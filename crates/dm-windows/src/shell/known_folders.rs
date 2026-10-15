//! Known-folder resolution for the user + public desktops (oracle: `DesktopPaths.Current`).
//! Paths are resolved via `SHGetKnownFolderPath`, never hardcoded (spec 07 §3).

use std::path::PathBuf;

use dm_domain::{PortError, PortResult};
use windows::core::{GUID, HSTRING};
use windows::Win32::Storage::FileSystem::GetDriveTypeW;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::WindowsProgramming::DRIVE_FIXED;
use windows::Win32::UI::Shell::{
    SHGetKnownFolderPath, FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Profile,
    FOLDERID_Programs, FOLDERID_ProgramData, FOLDERID_PublicDesktop, FOLDERID_RoamingAppData,
    KF_FLAG_DEFAULT,
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

/// The USER profile root (`FOLDERID_Profile`, e.g. `C:\Users\Melu`) — the recursive root of the
/// `explorer` surface's FOLDER items (ADR-0004 D7). Best-effort by contract: `None` on a
/// locked-down profile (the explorer folder surface is then skipped, like any other best-effort
/// root). The walk beneath it applies the hidden/system/reparse/denylist exclusions.
///
/// [WINDOWS-VERIFY] runtime.
pub fn profile_root() -> Option<PathBuf> {
    let path = known_folder(&FOLDERID_Profile).ok().flatten()?;
    path.is_dir().then_some(path)
}

/// The FIXED drives visible on this machine (ADR-0004 D7) — the `explorer` surface's DRIVE half,
/// e.g. `["C:\\", "D:\\", "E:\\"]`. Enumerated by probing every ASCII drive letter with
/// `GetDriveTypeW` and keeping only `DRIVE_FIXED` roots: removable / optical / network / RAM-drive
/// letters are transient or remote (a letter that can swap media or point at a share is the wrong
/// thing to style a per-machine registry key for), so they are never returned. A letter with no
/// volume reports `DRIVE_NO_ROOT_DIR` and simply drops out. Best-effort per-root doctrine: an empty
/// enumeration NARROWS the explorer surface instead of failing the scan.
///
/// [WINDOWS-VERIFY] runtime (drive-type filtering on the owner box: C:/D:/E: fixed).
pub fn visible_fixed_drives() -> Vec<String> {
    (b'A'..=b'Z')
        .filter_map(|letter| {
            let root = format!("{}:\\", letter as char);
            // SAFETY: `GetDriveTypeW` only reads the NUL-terminated path for the duration of the
            // call; the `HSTRING` outlives it.
            let drive_type = unsafe { GetDriveTypeW(&HSTRING::from(&root)) };
            (drive_type == DRIVE_FIXED).then_some(root)
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_fixed_drives_includes_the_system_drive_and_is_well_formed() {
        // Every Windows install has a fixed `C:` (the system drive), so this is a real smoke check
        // that the `GetDriveTypeW` probe + `DRIVE_FIXED` filter work on a live box. Each entry must
        // be a bare `<letter>:\` root — the exact shape `apply::drive::letter_from_path` parses back
        // out (a malformed entry would surface later as a wrong per-drive registry key).
        // [WINDOWS-VERIFY] runtime.
        let drives = visible_fixed_drives();
        assert!(
            drives.iter().any(|d| d == "C:\\"),
            "expected the fixed system drive among {drives:?}"
        );
        for d in &drives {
            assert_eq!(d.len(), 3, "drive root must be exactly X:\\, got {d:?}");
            assert!(d.ends_with(":\\"), "drive root must end in :\\, got {d:?}");
            assert!(
                d.starts_with(|c: char| c.is_ascii_alphabetic()),
                "drive root must start with a letter, got {d:?}"
            );
        }
    }
}
