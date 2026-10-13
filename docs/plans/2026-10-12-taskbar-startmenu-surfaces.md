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
            taskbar: boolean,          // default TRUE (owner decision 2026-10-12)
            startMenu: boolean }       // default TRUE (owner decision 2026-10-12)
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
11. **MSIX AppListEntry wrappers** (owner directive 2026-10-12 "solve Store-app
    icons"; recipe owner-verified on-box — see field findings below): the virtual
    Start entries become styleable through OWNED wrapper shortcuts, never package
    mutation:
    - **Scan synthesizes** one item per `FOLDERID_AppsFolder` entry with no
      file-backed `.lnk` in either Programs root: `kind = AppxShortcut` (free — the
      walker never emits it today), `surface = startMenu`, `parent = ""`,
      `path = <user Programs root>\<DisplayName>.lnk` (prospectual: the file does
      not exist yet; id = `ItemId::from_source_path` as usual). The AUMID is
      resolved LIVE — at scan for extraction, at write for creation — by display
      name → AppsFolder lookup. **No new `IconItemDto` field; schema stays 11.**
    - **Extraction — THE SOURCE GLYPH**: the wrapper's source must be the APP's own
      icon, never the wrapper's current `IconLocation` (a probe pointing at an
      arbitrary asset renders that arbitrary glyph — verified on-box: terminal glyph
      on WhatsApp). Rule: an item is `AppxShortcut` iff its shortcut args parse
      `shell:AppsFolder\<AUMID>` (classify.rs gains this parse; synthesized items
      take the AUMID from the AppsFolder display-name match) → extractor resolves
      `SHCreateItemFromRelativeName(FOLDERID_AppsFolder, AUMID)` and renders via
      `IShellItemImageFactory::GetImage` (reuse the `shell_item_image` → bytes
      tail). File-backed appsFolder shortcuts extract by AUMID too — falling back to
      their `IconLocation` would perpetuate whatever it already points at.
    - **AUMID round-trip on write**: `set_icon_location` (Load → SetIconLocation →
      Save) MUST preserve the property-store block carrying
      `System.AppUserModel.ID` — without the AUMID the running-window lookup falls
      back to the process exe and the pin goes green again. Assert in a test:
      set AUMID → rewrite icon → re-read AUMID == unchanged; fix the writer
      (re-apply the property after IShellLink::Save, or round-trip the store) if
      Save drops the block.
    - **State reader**: `AppxShortcut` with an absent wrapper skips
      `require_exists` (the System-CLSID precedent) → fresh + styleable; when the
      wrapper exists it reads like any `.lnk` (`IconRef`) so re-apply CAS keeps
      chaining. Fingerprint for "absent" = a dedicated sentinel (like the virtual
      recycle-bin state), stable both directions.
    - **Write**: `shell_link::create_appx_shortcut(path, aumid, icon)` —
      `IShellLink` (target `%SystemRoot%\explorer.exe`,
      args `shell:AppsFolder\<AUMID>`, icon location, ownership marker) then
      `IPropertyStore::SetValue(PKEY_AppUserModel_ID, GPS_READWRITE)`. The AUMID
      property is THE mechanism: Windows merges the wrapper with the virtual entry
      (2 rows → 1) and renders the wrapper's icon.
    - **[WINDOWS-VERIFY] Launch path — VERIFY ON BOX BEFORE SHIP**: the icon
      merge/render does NOT depend on the wrapper's target, but clicking DOES.
      On the owner's machine `explorer.exe shell:AppsFolder\<AUMID>` does not
      activate apps (3 attempts with Calculator → no process; the owner's
      busy-cursor loop when clicking the probe wrapper = this exact recipe),
      while `IApplicationActivationManager::ActivateApplication` (CLSID
      `45BA127D-10A8-46EA-8AB7-56EA9078943C`, **IID
      `2E941141-7F97-4756-BA1D-9DECDE894A3D`** — the widely-circulated
      `2E97EF3E-…` is wrong → E_NOINTERFACE) returns S_OK and the app stays up.
      The recipe is what Windows' own Store-app pins use → box-specific
      breakage correlating with the pre-existing COM registration damage
      (wallpaper bug `0x80040154`). Task 11 ships only after a wrapper is
      clicked end-to-end on-box; if the explorer route still fails, disclose
      (system repair `sfc /DISM` is out of repo scope) or re-target the
      wrapper — do NOT silently ship an entry that renders but cannot launch.
    - **Ledger / restore**: original = wrapper *Absent* → restore REMOVES the
      wrapper (the `file_wrapper` Absent→`remove_file` precedent) → the package
      icon returns. Zero package mutation; ADR-0024 Never-list intact.
    - **Elevation: never** — wrappers always go in the USER Programs root (the
      Start view merges both roots; verified per-user on-box), so the elevated
      helper gains no new op.
    - **Collisions**: a display name matching a walked `.lnk` stem → skip (the
      classic item covers it); two virtual apps sharing one display name → second
      skipped + scan warn (honest, logged).
    - **Tests**: synthesis (injected AppsFolder fixture), absent→styleable state
      reader, wrapper create + property-store + restore round-trip, ledger anchor
      round-trip, `commit_apply` path for `AppxShortcut` (unelevated), collision
      skips. Docs: ADR-0024 records the sanctioned wrapper recipe; the
      "silently-absent badge" follow-up is superseded by this task.

## [WINDOWS-VERIFY] field findings — live pass 2026-10-12 (owner box)

Verified ✅ (dev loop, `bun run tauri:dev` + `DESKMAKEOVER_ALLOW_UNSIGNED_HELPER=1`):

- Pipeline end-to-end with surfaces ON: scan 103 items / 104 jobs → bake → 6 chunks
  → CAS commit → `restart_shell` → **`commit ok=true`**, including the elevated
  ProgramData batch (user-scope items unelevated; ONE batched UAC approved).
- On-disk rewrites confirmed by hand: taskbar pins (`User Pinned\TaskBar\*.lnk`) and
  Start-menu shortcuts (user + common) carry `IconLocation` →
  `icon-assets/{hash}.ico`; re-applies stay idempotent (re-own via ledger).
- Initial hang root-caused: a fresh clone lacks the gitignored
  `public/dm_icon_wasm.wasm` (only `bun run wasm` writes it) → worker init rejects →
  the FIFO enqueue chain swallowed the rejection → zero `sourceReady` acks → `apply`
  waited forever on the last `bake:ok`.

Honest exceptions measured on-box:

- **MSIX package Start entries are AppListEntries with NO file-backed shortcut** →
  invisible to the file scan by design (Never-list: package-asset edits break the
  package signature). **7 of 97** Start entries here: *Click to Do · Configuración ·
  Copias de seguridad de Windows · Introducción · Reloj · Trello · WhatsApp*. The
  other 90 are `.lnk`-backed and restyle (owner spot-check: only Trello/WhatsApp
  reported unchanged).
- **[BREAKTHROUGH, owner-verified 2026-10-12] MSIX wrapper recipe works**: a probe
  `.lnk` in the user's Programs root with target `explorer.exe shell:AppsFolder\<AUMID>`
  + `System.AppUserModel.ID` = the package AUMID (via `SHGetPropertyStoreFromParsingName`
  with `GPS_READWRITE` → `PKEY_AppUserModel_ID` = `{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3},5`)
  + `IconLocation` = styled asset makes Windows MERGE it with the virtual AppListEntry
  (Get-StartApps: 2 rows → **1 row** after the AUMID set) and the merged entry renders
  **the wrapper's styled icon** (owner's eyes on the real Start menu, not just the
  enumeration). Deleting the wrapper restores the package icon → reversible, no package
  mutation. → Elevated to product scope as Task 11.
- Running-app pins keep the exe icon while the app runs (issue #8 / ADR-0024
  caveat). **Cold-pin repaint VERIFIED** (owner): Brave's pin had a styled `.lnk`
  on disk but showed the original with the app closed after the first restart
  (stale pin cache), and repainted to the styled icon after the next shell
  restart — no AppUserModelID exception observed on-box.
- **[2026-10-12 night] Probe teardown + launch-route finding**: the AUMID pin
  probe made the running WhatsApp taskbar button render OUR asset (mechanism ✅ —
  but an arbitrary placeholder glyph, terminal-derived: probes bypass extraction,
  the correct glyph needs Task 11's `IShellItemImageFactory` source, see spec).
  Launching then broke: clicking the probe wrapper (target = the standard
  `explorer.exe shell:AppsFolder\…` recipe) spun a busy cursor and never opened
  the app — same failure reproduces for Calculator via that route on this box,
  while direct `IApplicationActivationManager::ActivateApplication` activates
  WhatsApp fine (S_OK, stable process, window up). Probes (wrapper + pin)
  deleted → Start back to 1 row, pins back to the original set; package
  activation healthy. Lesson baked into Task 11: **two live shortcuts carrying
  the same AUMID + a broken launch route = user-visible breakage — probes on the
  real shell stay short-lived, and the wrapper's click-to-launch is a ship
  gate** (spec bullet added).
- Stray `File Explorer (2).lnk` in the pin folder = a manual user copy (19:16),
  not ours — owner to delete or keep.

- **Conflict test PASS (2026-10-12, CDP-driven UI)**: hand-edited `Brave.lnk`
  (`imageres.dll,-109`) → apply → CAS conflict on that pin only, its bytes and
  mtime untouched while the other ~100 items committed. The hand-edit rides the
  restore-job path (`restores=1` → `restore_skipped` → counted as a conflict)
  and also survives a later Restore (trust-first): skipped as a hand-edit until
  the item's fingerprint again equals `original_fingerprint`.
- **Restore test PASS (2026-10-12)**: apply → snapshot ledger → restore →
  **94/94 verifiable FileBytes byte-identical** to `original_anchor.bytes`
  (0 mismatches; the other 3 are files the user deleted long ago — rows healed
  as NotFound, as designed), wrapper `.lnk` sidecars removed and the originals
  un-hidden back to `Archive`, ledger 101 → 1 rows, `txn.log` 284 KB → 0
  (checkpointed), zero `icon-assets` residue except the unmanaged
  `File Explorer (2).lnk` stray.
- **Vanish-residue heal verified**: the 3 wrapped loose files were found
  Hidden|System with their sidecars already gone — a state the apply/restore
  code order cannot produce (external deletion: old crash artifact or AV/user
  cleanup). Recovery path: un-hide → Restore → the "already pristine" arm
  drops the rows → next apply re-wraps cleanly; the re-wrapped→restored cycle
  then came back byte-clean.
- **jpg wrap race (open)**: wrapping `wp14409295-….jpg` failed 4/4 with
  `0x80070005` at the attrs step — Explorer's thumbnail extraction locks the
  target inside the sidecar→hide window, and the rollback (sidecar removal)
  hits the same lock → `rollback-incomplete` txn. The recovery abort hit the
  same denial twice before landing (`icons … finalize degraded` WARNs), which
  **defers a restore until the abort clears** — correct but loud. Arithmetic
  match: the owner's historical `99 committed + restores=3 (+1)` gap is this
  same item. Disposition: marked **keep original** via the per-item override so
  applies run clean (`ok=true`, CTA In-sync); proposed fix (out of v1): retry
  `attrs::set` with backoff on `0x80070005` in `file_wrapper::apply`.
- **3 steady apply conflicts (open observation)**: every CDP apply reported
  `3 conflicts` (the owner's historical runs show none). The UI/event exposes
  the count only, so the ids are unattributed; every directly-inspected item
  is styled and the applies end `ok=true`/In-sync. Needs a conflict-id log
  line or event payload before it can be chased.
- **Test harness note**: WebView2 Runtime 150+ drops `WEBVIEW2_*` env vars for
  elevated hosts (WebView2Feedback #5645) — CDP for the elevated dev app was
  brought up via the HKLM policy `AdditionalBrowserArguments`
  (`--remote-debugging-port=9222`), removed again after the pass. Permanent
  dev-loop suggestion: a non-elevated CDP mode or a scripted test bridge.

Remaining checklist: hand-edit a pin → CAS conflict untouched ✅ (2026-10-12) ·
restore → byte-identical FileBytes on pins/Start ✅ (2026-10-12: 94/94) ·
uninstall residue `[pending]` · dev-loop hardening ✅ (worker posts
`ok:false`/`png:null` failure acks instead of swallowing init rejection;
TEMP-DEBUG markers removed; `**/target/**` vite watch ignore landed).

## Follow-ups (documented, out of v1)

- Resident auto-format covers the new surfaces (spec 07 §8.2 consent ladder).
- Per-surface style variants (v2 parking lot).
- `keepNewIconsStyled`-equivalent for pins created AFTER apply (the resident
  reconciler's watcher root gains the pin folder).
- Running-app exe-icon honesty: a "this app shows its .exe icon while running"
  badge once [WINDOWS-VERIFY] measures which app classes repaint.
- Common Start Menu per-item UAC-free path if the helper grows per-surface
  batching (today: one batched UAC, already disclosed).
- MSIX package Start entries: ~~non-stylable badge in the mirror~~ **superseded by
  Task 11** (owned wrappers make them styleable; the badge idea applies only to
  entries Task 11 cannot match).
