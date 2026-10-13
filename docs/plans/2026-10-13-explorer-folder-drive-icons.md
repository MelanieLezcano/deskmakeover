# Plan — Explorer module: folder + drive icons (ADR-0004 D7)

**Date:** 2026-10-13 · **Branch:** `feat/taskbar-startmenu-surfaces` (continues) → lands as the
Explorer module · **Status:** in progress

## Goal

Extend the icon look into **File Explorer**: style the **folder** icons a user sees when
browsing (via `desktop.ini`) and the **drive** icons (via the registry `DriveIcons` key), on a
new `explorer` surface with a per-surface toggle and full restore. This is the "资源管理器
module" already scheduled in `specs/00-roadmap.md` and bounded by **ADR-0004 Decision 7**.

## Scope decisions (owner 2026-10-13)

- **In scope:** folder icons (`desktop.ini`) + drive icons (registry `DriveIcons`, **never**
  `autorun.inf`). Same shape×colour engine, per-surface toggle, full restore.
- **Out of scope (constitutional, ADR-0004 D7):** global file-type association icons
  (ProgId `DefaultIcon`). Permanently banned — association-hijack signature, un-restorable.
- **Folder coverage:** every folder under the **user profile** (`FOLDERID_Profile`), subject to
  the exclusions below (owner chose "todas bajo tu perfil").

### Folder exclusions (safety + perf — owner informed)

Writing `desktop.ini` into every directory naively is catastrophic: the profile contains the
owner's repo (`target/`, `node_modules/` = tens of thousands of dirs). The explorer walk SKIPS:

- **Hidden + System attribute folders** — the whole `AppData` tree, `.git`-style hidden dirs, and
  anything app-managed. Matches what Explorer shows by default (owner never sees hidden dirs).
- **Reparse points / junctions / symlinks** — no writing `desktop.ini` *through* a link; also
  prevents cycles (OneDrive, redirected Documents). The apply side already refuses reparse
  points (`apply/folder.rs` APPLY-2); the scan mirrors it.
- **Build/VCS/cache trees** — `target`, `node_modules`, `.git`, `.venv`, `__pycache__`, `dist`,
  `build`, `.cache`, `Cargo.lock`-adjacent junk dirs. A denylist by directory name.
- **Depth + count cap** — a max walk depth and a max folder count so a freak tree can never
  stall the scan (mirrors the existing read caps).

## Architecture (what exists vs what's new)

| Piece | Status |
|---|---|
| `desktop.ini` folder apply/restore (`apply/folder.rs`, `FolderIconWriter`) | ✅ exists — reused |
| Folder state read (`state_reader.rs` `SurfaceState::Folder`) + original icon (`source.rs`) | ✅ exists — reused |
| `ItemSurface::Explorer` | 🆕 domain enum variant |
| `ItemKind::Drive` (+ bucket) | 🆕 domain |
| Profile root + visible-drive enumeration (`known_folders.rs`) | 🆕 |
| Explorer scan walk (profile folders + drives) | 🆕 `shell/scan.rs` |
| Drive icon extraction (current `DriveIcons`/default) | 🆕 `source.rs` |
| Drive state fingerprint (`state_reader.rs`) | 🆕 |
| Drive apply/restore via `DriveIcons` registry (HKLM) | 🆕 `apply/drive.rs` + elevated helper cmd |
| `explorer` surface toggle (frontend `Surfaces`, serialize, UI) | 🆕 |
| Tests + gates | 🆕 |

## Phases

### Phase A — Folders (user-scope, in-host; the visible value)

1. **Domain:** `ItemSurface::Explorer`; `kindBucket(Folder)` already `Folder`. No `Drive` yet.
2. **Enumerate:** `known_folders.rs::profile_root()`; explorer walk in `shell/scan.rs` —
   recursive, exclusions above, `surface=Explorer`, `kind=Folder`.
3. **Scan wiring:** emit explorer Folder items; classify; extraction reuses folder icon path.
4. **State/apply/restore:** reuse existing folder mechanics (surface only gates participation,
   not the write mechanism). Verify in-host desktop.ini writes for profile folders.
5. **Frontend:** add `explorer` to `Surface`/`Surfaces`, `surfaceParticipates`, serialize,
   `DEFAULT_SURFACES`, toggle UI.

### Phase B — Drives (elevated, registry `DriveIcons`)

6. **Enumerate:** `known_folders.rs::visible_fixed_drives()` (`GetLogicalDriveStringsW` +
   `GetDriveTypeW`, fixed only).
7. **Domain:** `ItemKind::Drive` + bucket (`App`), `is_styleable`.
8. **Extract/state:** read current `DriveIcons\<L>\DefaultIcon` (or shell default) for the
   original + fingerprint.
9. **Apply/restore:** `apply/drive.rs` writes/removes `HKLM\…\Explorer\DriveIcons\<L>\DefaultIcon`;
   routed through the **elevated helper** (new command: set/delete a DriveIcons value).
10. **Wires + toggles** for the `explorer` surface covering drives.

### Gates & docs

- `bun test`, `cargo test --workspace`, `bun x tsc -b`, `bun run lint`, `bun run check:bindings`,
  banned-colors test — green before any commit.
- On-box verification: folders under profile restyled (visible + AppData skipped), a fixed drive
  restyled, two clean apply/restore cycles.
- ADR-0004 amendment (Explorer module landed) + roadmap tick.

## Status log

- [x] Investigation + scope locked (owner).
- [x] Phase A folders (domain `ItemSurface::Explorer` + `SurfacesJson.explorer` + `SurfaceDto` +
      frontend surface toggle + profile walk with exclusions + Desktop-overlap dedup). All gates
      green: `cargo test --workspace`, `bun test` 670, `tsc`, `lint`, `check:bindings`, plus a new
      `the_explorer_walk_emits_folders_recurses_and_honors_exclusions` scan test.
- [ ] Phase B drives.
- [ ] Gates + on-box + docs.
