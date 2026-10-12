---
plan: taskbar-startmenu-surfaces
created: 2026-10-12
owner-approved: none — fork draft (ADR-0024 status: Proposed, NOT owner-disposed)
status: proposed
touches: dm-domain, dm-contracts, dm-windows, dm-operations, src-tauri, src
---

# Style taskbar pins and Start menu shortcuts (shell surfaces)

## Problem

DeskMakeover styles ONLY the desktop. `WindowsScanner::scan_blocking`
(`dm-windows/src/shell/scan.rs`) reads the two desktop roots + Recycle Bin +
CLSID virtuals and nothing else. The community ask (upstream issue #8, open
enhancement): icons restyled on the desktop still show the ORIGINAL icon on the
taskbar — Windows resolves a taskbar button's icon from the PINNED shortcut
(`%APPDATA%\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\*.lnk`)
or from the running exe's resources, never from the desktop `.lnk` DeskMakeover
wrote. Start menu entries are a third independent copy of the same shortcut with
the same problem. There is no supported override API for running-window icons
(see §Non-goals); the levers that EXIST are the shortcut `IconLocation` writes we
already own.

Dev-box inventory (2026-10-12, the verification corpus): 3 taskbar pins (Brave,
File Explorer, Windows PowerShell) · 20 user Start menu `.lnk` across 6
subfolders · 72 all-users Start menu `.lnk` · 5 desktop items.

The write machinery does NOT need a new applier: a pinned/start `.lnk` is an
ordinary `ItemKind::Shortcut` and rides `apply/shortcut.rs` →
`shell_link::set_icon_location`, `RestoreAnchor::FileBytes`, the CAS ledger and
the existing elevated batch (a `ProgramData` ancestry already classifies
`PrivilegedScope::ProgramData`). The delta is SCAN + CONTRACT + PARTICIPATION +
MIRROR, plus the ADR-0024 amendment reversing the "taskbar is scenery" clauses.

## Design

### Surface model — one scan, three labeled surfaces

`IconItemDto` gains `surface: SurfaceDto` (`"desktop" | "taskbar" |
"startMenu"`, `#[serde(default)] = desktop` for downgrade tolerance) and
`parent: String` (Start menu subfolder relative path, `""` at root; `""` for
desktop/taskbar). Bridge schema **10 → 11**; regenerate bindings.

Identity stays `ItemId::from_source_path("filesystem", path)` — a pin and a
desktop shortcut of the same app are TWO items with independent fingerprints,
ledger rows and masters. The content-addressed store dedupes the pixels: same
look ⇒ same `.ico` hash ⇒ one asset, N ledger rows. Display-name duplication is
expected (both `Brave`), and is exactly why positions stop being name-matched
across surfaces (§Mirror).

Scan roots, all resolved through `known_folders.rs` (`SHGetKnownFolderPath` +
labeled suffix, same rule as `desktop_roots_labeled` — never a full hardcoded
path):

| Surface | Root | Walk |
|---|---|---|
| desktop | existing | flat (unchanged) |
| taskbar | `FOLDERID_RoamingAppData` + `\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar` | flat |
| startMenu | `FOLDERID_Programs` (user) + `FOLDERID_CommonPrograms` (all-users) | RECURSIVE, depth cap 8, skip reparse points |

The pin folder itself is Hidden+System; the EXISTING per-entry checks
(`is_ignored_entry`, `has_system_attribute`) run on entries INSIDE a root and
stay as-is — no bypass, entries are ordinary `.lnk`s. `desktop.ini` skipping
already applies. Common-Start-menu items classify `PrivilegedScope::ProgramData`
by ancestry → they ride `apply_privileged_batch` (kind `Shortcut`/`AppxShortcut`
covers every Start menu `.lnk`) → ONE UAC for the batch, disclosed by the
existing consent sheet. No new elevation code.

All surfaces ALWAYS scan (mirroring kindPolicy semantics): a disabled surface
renders its tiles as-original and ships no master (RESTORE-FIRST); enabling a
surface never requires a rescan.

### Participation axis — `surfaces`, orthogonal to `kindPolicy`

```ts
surfaces: { desktop: true /* always-on, not user-toggleable */,
            taskbar: boolean,          // default FALSE (opt-in)
            startMenu: boolean }       // default FALSE (opt-in)
```

Persisted through the SAME channel as `kindPolicy` (`icons.setLook` →
`IconPersistedDto` → module state), and — same rule as spec 06 §6 — **never in
`ConfigDto`** (surfaces are participation, not look; presets/history must stay
clean). Normalizers whitelist the key on BOTH bridge sides (G11: a closed
whitelist — `src/lib/kind-policy.ts` + the Rust normalizer in
`dm-operations/settings_store.rs`), or the key is silently dropped.

Cascade for a tile's effective config (outermost first):

```
styleable:false  >  per-icon override  >  kindPolicy bucket  >  surface gate
```

The surface gate is the outermost PARTICIPATION gate: a disabled surface's
items never enter `bakeSet`. Bake set (frontend `stores/icons.ts apply()`):

```
items.filter(i => surfacesOn(i.surface)
               && kindParticipates(bucket(i))
               && !effectiveTileConfig(i).showOriginal)
```

…with `restoreIds` for kept-original overrides unchanged.

### Mirror — real pins in the strip, a Start panel view

- **Taskbar strip**: the decorative `TASKBAR_PINNED` fixture entries
  (`taskbar-icons.tsx`, 7 hardcoded glyphs) are REPLACED by the real scanned
  pins (styled previews through the normal Tile renderer — app chrome, NOT the
  OS-mirror palette; tray/clock/Start-flag scenery stays untouched). Pins with
  `styleable:false` render full-fidelity original (honest, never dimmed) with
  no context menu — existing §3.5 rule.
- **Start menu**: a new surface chip in the canvas toolbar (`桌面 | 任务栏 |
  开始菜单` → `Escritorio | Barra de tareas | Menú Inicio`) swaps the canvas to
  a Start-panel mock that lists the REAL Start menu items (grouped by `parent`)
  as ordinary tiles. The switch is OUR chrome (canvas toolbar), so the
  decorative Start flag itself stays non-interactive — D7's "no interactivity"
  is preserved for scenery; only tiles gain behavior.
- **Tile behavior is uniform across surfaces**: hover try-on, press-to-peek,
  owned-verb context menu (保留原样 / 跟随全局 / 单独配色), exception badges,
  undo/redo steps. Position slots: `live_slots` name-matching in
  `icon_host/scan.rs` applies **only to `surface == desktop`** (a desktop `Brave`
  and a pinned `Brave` would otherwise share one slot — G2); taskbar/startMenu
  tiles lay out by strip order / `parent` grouping in the component.

### Apply, refresh, restore — unchanged machinery

- Chunked bake (`applyBakedBegin/Chunk/Commit`), session caps, per-item txns,
  CAS anchors at scan fingerprint, journal + ledger + recovery: **zero changes**.
- Refresh after a mutating commit keeps `restart_shell()` +
  `purge_icon_caches()` (the proven path; `SHChangeNotify` alone does not repaint
  the taskbar) — the existing test
  `every_desktop_mutating_op_restarts_the_shell_but_an_idempotent_reapply_does_not`
  keeps its intent. ConsentSheet copy gains the per-surface counts (i18n
  en + zh-hans in lockstep): "also styles N taskbar pins and M Start menu
  shortcuts — your shortcuts' icons only, never the apps' files".
- `icons.restore()` walks every ledger row regardless of surface — one-click
  full restore covers the new surfaces with no code.

### Non-goals (v1)

- **Exe resource rewriting — never.** Editing `app.exe` icons breaks
  Authenticode signatures and is out of the reversibility envelope (a restored
  `.lnk` is byte-replay; a rewritten exe is not). Running-window buttons whose
  Windows renders from the exe keep the original icon while the app runs —
  documented honestly `[WINDOWS-VERIFY]` (the issue-#8 field report shows pins
  DO repaint for many apps after shell restart).
- **UWP package assets** stay immutable (shortcut-level styling only —
  existing spec 06 §6 rule).
- **Resident/background auto-format** (spec 07) keeps desktop-only; surfaces
  join it in a follow-up behind the same consent ladder.
- **Per-surface looks** (different style per surface) — v2 parking lot.
- Windows' own taskbar chrome (tray icons, running indicators) — scenery forever.

## Ports / DTOs (dm-domain, dm-contracts)

```rust
// dm-contracts/src/icons.rs
#[derive(Default)] #[serde(rename_all = "camelCase")]
pub enum SurfaceDto { #[default] Desktop, Taskbar, StartMenu }   // serde literals tested
// IconItemDto += surface: SurfaceDto (default), parent: String (default "")
// BRIDGE_SCHEMA_VERSION 10 → 11 (src/bridge/types.ts), bindings regenerated
```

No new ports: scan is a `WindowsScanner` internal; apply/restore/scope reuse
every existing port.

## Task list

1. **Docs**: ADR-0024 (amends ADR-0015 D7 · spec 06 §4 · spec 01 canvas clause)
   + this plan — the only commit that lands before code.
2. **dm-contracts**: `SurfaceDto` + `IconItemDto.{surface,parent}` + serde
   defaults + literal tests; schema 11. `bun run gen:bindings` +
   `cargo test -p deskmakeover-desktop --test bindings` green.
3. **dm-windows**: `known_folders.rs` roots (`taskbar_pin_root`,
   `start_menu_roots_labeled`) + `scan.rs` surface-labeled walk (flat pin,
   recursive start menu, depth cap, per-root label); `classify_entry` unchanged;
   unit tests on fs fixtures (pin folder, nested start folders, desktop.ini in
   roots, reparse skip).
4. **src-tauri `icon_host`**: surface-aware assembly — `live_slots` +
   `synthetic_layout` desktop-only; other surfaces emit `surface`/`parent`
   (x/y = 0, frontend-owned layout); persisted `surfaces` state through
   `getPersisted`/`setLook` with the Rust-side normalizer whitelist.
5. **dm-operations**: normalizer whitelist for `surfaces` (settings_store);
   confirm `scope.classify` ancestry hit for CommonPrograms paths (test:
   `C:\ProgramData\Microsoft\Windows\Start Menu\Programs\…` → `ProgramData`);
   NO other logic changes.
6. **Frontend store**: `surfaces` state + setters + normalizers (G11, both
   sides), bakeSet gate, undo steps for surface flips (one step each), draft
   snapshot/resume (surfaces ride the same draft as kindPolicy).
7. **Frontend UI**: participation rows (「参与美化的类型」 grows a SURFACES
   group with live counts), canvas surface chips, strip real-pin tiles, Start
   panel mock view, i18n en + zh-hans lockstep.
8. **Gates**: `bun test` · `bun run lint` · `tsc -b` · `cargo test --workspace`
   · bindings drift · banned-colors allowlist (expect NO change: styled tiles
   ride the shared Tile component, strip files keep only scenery literals —
   confirm, adjust the allowlist only if the test scans rendered subtree).
9. **[WINDOWS-VERIFY] live pass** (owner-supervised, per
   `docs/verification/`): scan shows 3+20(+72) · preview mirrors pins · apply →
   pins repaint after shell restart (running vs cold apps noted honestly) ·
   Start menu entries restyle (user + all-users UAC batch) · hand-edit a pin →
   conflict, untouched · restore → byte-identical originals (FileBytes) ·
   uninstall residue check.
10. **Upstream**: per the 2026-07-21 community-proposal panel record — spec-scoped
    PR splits (contracts+scan · store+UI · docs), each PR states touched spec
    sections + gates run + the 5 owner invariants; adversarial review on
    ledger/CAS/parity. Expect deferral (write-surface reliability is the owner's
    dominant ship risk) — the fork carries the feature until then.

## Follow-ups (documented, out of v1)

- Resident auto-format covers the new surfaces (spec 07 §8.2 consent ladder).
- Per-surface style variants (v2 parking lot).
- `keepNewIconsStyled`-equivalent for pins created AFTER apply (the resident
  reconciler's watcher root gains the pin folder).
- Running-app exe-icon honesty: a "this app shows its .exe icon while running"
  badge once [WINDOWS-VERIFY] measures which app classes repaint.
- Common Start Menu per-item UAC-free path if the helper grows per-surface
  batching (today: one batched UAC, already disclosed).
