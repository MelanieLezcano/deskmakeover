//! Known-folder resolution for the user + public desktops (oracle: `DesktopPaths.Current`).
//! Paths are resolved via `SHGetKnownFolderPath`, never hardcoded (spec 07 §3).

use std::path::PathBuf;

use dm_domain::{PortError, PortResult};
use windows::core::GUID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    SHGetKnownFolderPath, FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Programs,
    FOLDERID_ProgramData, FOLDERID_PublicDesktop, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
};

/// The desktop roots LABELED by identity: `(user desktop, public desktop)`. The USER desktop is
/// MANDATORY — a failed lookup / missing directory is an error, never silently skipped (codex
/// R-vanish P2: positional `desktop_roots()` let a failed user-desktop resolution promote the
/// Public root to index 0, so a broken user desktop masqueraded as a successful empty/public-only
/// scan). The Public root stays best-effort (`None` on locked-down editions).
///
/// [WINDOWS-VERIFY] runtime.
pub fn desktop_roots_labeled() -> PortResult<(PathBuf, Option<PathBuf>)> {
    let user = match known_folder(&FOLDERID_Desktop) {
        Ok(Some(path)) if path.is_dir() => path,
        Ok(Some(path)) => {
            return Err(PortError::Io(format!(
                "the desktop folder {} is not an accessible directory",
                path.display()
            )))
        }
        Ok(None) => return Err(PortError::Io("the desktop folder did not resolve".into())),
        Err(e) => return Err(e),
    };
    let public = match known_folder(&FOLDERID_PublicDesktop) {
        Ok(Some(path)) if path.is_dir() => Some(path),
        _ => None,
    };
    Ok((user, public))
}

/// The existing desktop roots (user first, then public). Missing roots are skipped.
///
/// [WINDOWS-VERIFY] runtime.
pub fn desktop_roots() -> PortResult<Vec<PathBuf>> {
    let mut roots = Vec::new();
    for id in [&FOLDERID_Desktop, &FOLDERID_PublicDesktop] {
        // One folder's lookup failing (e.g. FOLDERID_PublicDesktop unresolved on a
        // locked-down profile/edition) must NOT discard an already-resolved user Desktop
        // and fail the whole scan — the doc promise "missing roots are skipped" has to hold
        // for an API-level error, not only a physically-absent folder. Skip that root. (SHELL-1)
        match known_folder(id) {
            Ok(Some(path)) if path.is_dir() => roots.push(path),
            Ok(_) => {}
            Err(_) => continue,
        }
    }
    Ok(roots)
}

/// The §14 privileged-scope roots: `(Public Desktop, ProgramData)`, resolved via
/// `SHGetKnownFolderPath` (spec 07 §14, never hardcoded). Fail-closed contract: `None` when
/// EITHER folder does not resolve — the caller must then run with `ScopeRoots::Unresolved`
/// (defer everything) rather than a partial gate that silently fails open on the missing half.
///
/// [WINDOWS-VERIFY] runtime.
pub fn privileged_roots() -> Option<(PathBuf, PathBuf)> {
    let public = known_folder(&FOLDERID_PublicDesktop).ok().flatten()?;
    let programdata = known_folder(&FOLDERID_ProgramData).ok().flatten()?;
    Some((public, programdata))
}

/// The taskbar PIN root for the taskbar surface (ADR-0024): the folder Explorer stores the user's
/// pinned taskbar shortcuts in. Resolved from `FOLDERID_RoamingAppData` + the documented suffix —
/// never a fully hardcoded path (spec 07 §3). Best-effort by contract: `None` when the folder does
/// not resolve, so a locked-down edition narrows the SURFACES covered instead of failing the scan.
///
/// [WINDOWS-VERIFY] runtime.
pub fn taskbar_pin_root() -> Option<PathBuf> {
    let base = known_folder(&FOLDERID_RoamingAppData).ok().flatten()?;
    let root = base.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");
    root.is_dir().then_some(root)
}

/// The Start menu program roots for the start-menu surface (ADR-0024): `(user, all-users)`, both
/// plain `SHGetKnownFolderPath` known folders (no suffix). Order is stable — user first, so an
/// all-users failure can never displace it (the R-vanish labeling lesson, per root). Both legs are
/// best-effort: a missing root narrows the surface instead of failing the scan; the all-users leg
/// classifies `PrivilegedScope::ProgramData` downstream and rides the existing elevated batch.
///
/// [WINDOWS-VERIFY] runtime.
pub fn start_menu_roots_labeled() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for id in [&FOLDERID_Programs, &FOLDERID_CommonPrograms] {
        match known_folder(id) {
            Ok(Some(path)) if path.is_dir() => roots.push(path),
            Ok(_) => {}
            Err(_) => continue,
        }
    }
    roots
}

/// The USER Programs root (`FOLDERID_Programs`) — the directory a Store/MSIX wrapper `.lnk` is
/// CREATED into (Task 11, ADR-0024). User scope by construction, so a wrapper apply never routes
/// through the elevated helper. Best-effort by contract: `None` on a locked-down profile (the
/// AppsFolder surface is then skipped, exactly like any other best-effort root).
///
/// [WINDOWS-VERIFY] runtime.
pub fn user_programs_root() -> Option<PathBuf> {
    let path = known_folder(&FOLDERID_Programs).ok().flatten()?;
    path.is_dir().then_some(path)
}

fn known_folder(id: &GUID) -> PortResult<Option<PathBuf>> {
    // SAFETY: `SHGetKnownFolderPath` allocates a `PWSTR` that we own and free with
    // `CoTaskMemFree`; `to_string` reads the NUL-terminated buffer before it is freed.
    unsafe {
        let pwstr = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)
            .map_err(|e| PortError::Com(e.to_string()))?;
        if pwstr.is_null() {
            return Ok(None);
        }
        // Free the buffer BEFORE propagating a decode error, or the `?` early-return leaks it.
        let text = pwstr.to_string();
        CoTaskMemFree(Some(pwstr.0 as *const core::ffi::c_void));
        Ok(Some(PathBuf::from(text.map_err(|e| PortError::Com(e.to_string()))?)))
    }
}
