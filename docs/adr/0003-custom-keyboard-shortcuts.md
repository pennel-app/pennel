# Store custom shortcuts in a lenient settings field beside the legacy hotkey fields

Status: accepted by the owner on 10 November 2026 (redesign task T10a, issue #209; all six recommendations accepted). Implementation is staged through T10a–T10g; the acceptance gates below remain separate from source completion.

## Context

The desktop redesign (handoff section 3, screens 26 to 29) makes every shortcut editable: a recorder in Settings → Dictation and a Settings → Shortcuts page with thirteen rows. Today the keys are fixed. `HotkeyTrigger` in `core/src/settings.rs` is a five-value enum, ⌘⇧R is hard-coded in `ShortcutTap.swift` and `record_shortcut.rs`, the in-app keys are written twice (`desktop_commands.rs`, `desktop-chrome-state.ts`), and labels are formatted separately in TypeScript and Rust. Cancel dictation and a global Open Pennel do not exist.

The code sets four limits on the model. Startup parses settings strictly (`Settings::load_strict`, then setup aborts), so an enum value an older build does not know stops that build from starting. The dictation hooks pass every key on to the front app on macOS, Windows and X11. The gesture cancels a hold on any other key and cannot cancel hands-free. While a new shortcut is being recorded, the live hooks, the ⌘⇧R tap and the Mac menu would all act on the keys. Routine CI is also macOS-first, so it does not compile the Windows and Linux hotkey code.

## Decision

### Settings shape

`Settings` gains one field. `hotkey` and `record_shortcut_enabled` stay, and `HotkeyTrigger` never gains a value.

```rust
pub struct ShortcutBinding { pub mods: Vec<String>, pub code: Option<String>, pub key: Option<String> }

#[serde(default, deserialize_with = "lenient_shortcuts", skip_serializing_if = "HashMap::is_empty")]
pub shortcuts: HashMap<String, Option<ShortcutBinding>>,
```

The map is sparse, from action id to binding. A missing id means the legacy field or the default applies. `null` means the owner cleared the shortcut and the row reads None. An example from a Mac with an AZERTY keyboard:

```json
{
  "hotkey": "fn",
  "record_shortcut_enabled": false,
  "shortcuts": {
    "dictate": { "mods": ["ctrl", "alt"], "code": "Space", "key": "Space" },
    "record": { "mods": ["ctrl", "alt"], "code": "KeyR", "key": "r" },
    "open_pennel": { "mods": ["ctrl", "alt"], "code": "KeyQ", "key": "a" },
    "find": null
  }
}
```

`mods` holds `ctrl`, `alt`, `shift`, `meta` (⌘, Windows or Super), their `left_` and `right_` forms, and `fn` (macOS only). A binding without a key may name sides, so Right Option stays different from Left Option. A binding with a key never names sides, because the registration APIs and today's ⌘⇧R tap cannot tell them apart.

A key is named twice, the way the DOM names it. `code` is how the platform's shortcut layer identifies the key, written with W3C code names. On macOS that is the key position, since the tap sees virtual keycodes. On Windows and X11 it is the key as the active layout names it, since `RegisterHotKey` takes virtual keys and the X11 grab starts from a keysym (T10e confirms the X11 part against the plugin source). `key` is the character the key typed without modifiers when it was recorded, or a name such as `Escape` or `F5`. Global bindings match on `code`. In-app bindings match on `key`, as the Mac menu and the keydown handler do today. Displays, conflict checks and menu accelerators use `key`. In the example the owner pressed ⌃⌥ and the key printed A: that key fires, and the row shows ⌃⌥A. Settings are a per-machine file and are not synced, so a `code` never has to mean the same thing on two systems.

### Actions

| Action | Id | Scope | macOS default | Windows and Linux default | Editable |
| --- | --- | --- | --- | --- | --- |
| Dictate | `dictate` | Any app, hold | fn | Right Ctrl | Yes |
| Command for selected text | none | Follows Dictate | Shift with Dictate | Shift with Dictate | No |
| Cancel dictation | `cancel` | While the pill listens | Esc | Esc | Yes |
| Next speech model | `next_model` | While the pill listens | Left Option (question 2) | None (question 2) | Yes, from T11b |
| Start or stop a recording | `record` | Any app, one press | ⇧⌘R | Ctrl+Shift+R | Yes |
| Open Pennel | `open_pennel` | Any app, one press | None (question 1) | None | Yes |
| New recording | `new_recording` | In Pennel | ⌘N | Ctrl+N | Yes |
| Search | `find` | In Pennel | ⌘F | Ctrl+F | Yes |
| Go to Dictation, Meetings, History | `dictation`, `meetings`, `history` | In Pennel | ⌘1, ⌘2, ⌘3 | Ctrl+1, Ctrl+2, Ctrl+3 | Yes |
| Show or hide the sidebar | `toggle_sidebar` | In Pennel | ⌃⌘S | Ctrl+Shift+S | Yes |
| Settings | `settings` | In Pennel | ⌘, | Ctrl+, | Yes |

The in-app ids are the existing `DesktopCommand` ids. Close window and Quit follow the system and appear as fixed text. Apart from the two new actions, the defaults are today's keys, so nothing changes for someone who never opens the page. Open Pennel runs the existing `open_default`.

### Precedence over the legacy fields

The effective binding of an action is its `shortcuts` entry if there is one, otherwise the value derived from the legacy field, otherwise the default. The derivation copies what `HotkeyMonitor.swift` and `TriggerTracker` do with the enum today:

| `hotkey` | Dictate on macOS | Dictate on Windows and Linux |
| --- | --- | --- |
| `fn` | fn | Right Ctrl |
| `right_control` | Right Ctrl | Right Ctrl |
| `right_option` | Right Option | Right Alt |
| `right_command` | Right ⌘ | Right Windows or Super |
| `control_super` | fn | Ctrl with Windows or Super |

Record derives from `record_shortcut_enabled`: off is None, on is the default.

Every write goes through one core function. It also rewrites the legacy fields, so an older build always finds values it can parse:

- A Dictate binding equal to a preset sets `hotkey` to that preset and removes the entry. Any other Dictate binding, or None, stores the entry and sets `hotkey` to the platform default.
- Record at its default sets the flag and removes the entry. Record cleared unsets the flag and removes the entry. A custom Record binding stores the entry and unsets the flag, so an older build does not take ⌘⇧R back (question 4).
- Any other action stores its entry unless the binding equals the default.

`shortcuts` therefore holds only what the old fields cannot say. Until the recorder ships in T10f nothing writes `shortcuts`. The preset buttons (`views/dictation.ts`) and the Meetings toggle (`views/settings.ts:226`) keep writing the old fields, and a reader gets the same answer whether it has moved to effective bindings or still reads the old fields. T10f moves both controls to the core function in the same PR that adds the recorder. As a guard, `update_settings` drops the `dictate` or `record` entry when a save changes the matching legacy field and leaves the entry alone, so a legacy writer is never silently ignored.

### Lenient parsing

The stored shape holds strings only, with no enums, and `lenient_shortcuts` never returns an error. A `shortcuts` value that is not an object reads as empty. An entry that is neither `null` nor a binding object is skipped. Unknown action ids and unknown tokens are kept and written back, so a file from a newer build survives a round trip through this one. A binding this build cannot resolve counts as absent for that action.

### Validation and conflicts

Core validates in `set_shortcut(action, change, platform, replace)` and returns a structured problem: `conflict` with the other action's id, `reserved`, `not_allowed` or `unsupported`, each with a reason code. The frontend supplies the English and Dutch text.

- Dictate takes modifiers only, or a key with at least one of Ctrl, Option or Alt, ⌘ or Windows, or a function key alone. Shift is refused, because Shift with Dictate is Command.
- Cancel dictation and Next speech model take Esc or a function key alone, one modifier alone except Shift, or a combination. They can be this bare because they exist only while the pill listens.
- Record and Open Pennel take a key with at least one of Ctrl, Option or Alt, ⌘ or Windows, or a function key alone.
- In-app actions take a key with ⌘ on macOS or Ctrl elsewhere, or a function key alone. The key must be one a menu accelerator can express.
- `fn` is accepted on macOS and only without a key, because macOS sets the fn flag on arrow and function keys by itself.
- A short reserved list per platform is refused: quit, close, hide, minimise, the app switcher, Spotlight, the edit keys (copy, cut, paste, select all, undo, redo), Ctrl+Alt+Del and Win+L.

All actions share one namespace, so one set of keys belongs to one action. Two bindings with a key conflict when their modifiers match with sides ignored and their `code` or `key` match. Two modifier-only bindings conflict when one set contains the other, where `alt` contains `right_alt`. Command occupies Dictate with Shift. This rule catches ⌥ for Next speech model against the Right Option preset.

On a conflict nothing is saved and the alert names the other action. "Use it here" calls the function again with `replace`. In one save the keys go to this action and the other action is set to `null`, so it reads None and shows its reset arrow. The button is absent when the other action is Command or the keys are reserved. After a hand edit, an entry that fails these rules is ignored at load and the action uses its default. If two stored entries collide, the first in table order keeps the keys.

The reset arrow compares the effective binding with the platform default, so a preset chosen through `hotkey` shows it too. "Reset all" empties `shortcuts` and restores both legacy fields.

### What each platform can do

Core holds this table, keyed by platform, so routine CI tests all four columns on a Mac. The shell adds runtime facts (missing Accessibility, a failed registration) and publishes one row per action in `AppInfo.shortcuts`: effective binding, default, label, editable, availability and reason. An unavailable row stays visible and disabled with its reason, and `docs/desktop.md` records the gap.

| Action | macOS | Windows | Linux X11 | Linux Wayland |
| --- | --- | --- | --- | --- |
| Dictate, modifiers only | Yes, with fn | Yes | Yes | No. The reason names `pennel --toggle-dictation` |
| Dictate with a key | Yes | Yes | No in this version (question 3) | No |
| Cancel dictation | Yes | Yes | Yes | No |
| Next speech model | Yes | Yes | Yes | No |
| Start or stop a recording | Yes | Yes | Yes | No. The reason names `pennel --toggle-meeting` |
| Open Pennel | Yes | Yes | Yes | No. The reason says to bind `pennel`, which already opens the window |
| In Pennel | Yes | Yes | Yes | Yes |

On macOS every row except the last needs Accessibility, as dictation does today.

### Routing and withholding

| Scope | macOS | Windows | Linux X11 |
| --- | --- | --- | --- |
| Dictate, modifiers only | Listen-only tap in `HotkeyMonitor`, as today | Low-level hook, as today | XRecord, as today |
| Dictate with a key | Swallowing tap | The hook withholds the key | Refused |
| Cancel, Next model | Swallowing tap, matched only while the pill listens | The hook withholds, only while the pill listens | Plugin grab, registered when listening starts and released when it ends |
| Record, Open Pennel | Swallowing tap | `RegisterHotKey` through the plugin, as today | X11 grab through the plugin, as today |
| In Pennel | Menu accelerators | Keydown handler | Keydown handler |

On macOS the one-press combinations use the swallowing tap, generalised from one hard-coded combination to a table. A registration API (Carbon hot keys, which the global-shortcut plugin wraps) would work without Accessibility and should keep working during secure input. It cannot see fn, sides or a bare modifier, so the tap would remain for Dictate and for recording a shortcut, and the plugin is a Windows and Linux dependency today. The tap already ships for ⌘⇧R, so extending it is less code and keeps conflicts in one place. The cost is that tap shortcuts stop while a password field holds secure input, as dictation does now.

A matched key never reaches the front app. Once a key-down is withheld, its repeats and its key-up are withheld until the key is released, even when a modifier is let go first. The swallowing tap is installed only while its table is non-empty, because an active tap sits in the path of every keystroke. Its callback stays a table lookup that returns at once.

A bare modifier bound to Next speech model fires on release when no key or click came while it was down, so the monitor's event mask gains mouse-down. It is read by the listen-only layer and is not withheld. On Windows the existing mask-key trick for the Windows key extends to a bare Alt, which would otherwise open the front app's menu bar.

In-app keys come from one table. On macOS it sets the seven menu accelerators through `MenuItem::set_accelerator`. On Windows and Linux `desktopShortcut` reads it. Both match `key`, as they do today.

### Gesture changes

- `Key::Other` carries the platform key code, and `TriggerTracker` follows a hold binding in place of a `HotkeyTrigger`. It is engaged when every modifier and the key are down. The old constructor stays as a mapping from the enum until T10e, so the Windows and Linux files compile unchanged.
- The tracker also decides what to withhold. `hotkey/mod.rs` compiles on macOS, so routine CI tests that logic, and the Windows hook only returns the answer.
- Interrupt is raised only when Dictate is modifiers only. The binding's own key, its repeats and a Cancel or Next model key never raise it.
- `HotkeyGesture::cancel()` is new. From a hold, hands-free or a manual session it returns `Cancel`, then waits for release if the trigger is still down. Any other key still does not stop hands-free.
- The timings stay (0.25 s tap, 0.35 s double-tap window, 600 s limit) and apply to a binding with a key. Shift during a hold still means Command.
- With Dictate cleared there is no hold trigger. The button, the tray item and `--toggle-dictation` still use `manual_toggle`, and the Command row reads None.
- `HotkeyEvent::MeetingShortcut` becomes `HotkeyEvent::Action(id)`, and the `record_shortcut_enabled` check in `dictation.rs` goes, since a cleared Record binding is no longer in the table.

### Recording a shortcut

Recording uses a native call, named shortcut capture because `capture.rs` already means the render profile. The frontend calls `shortcut_capture_start(action)` and `shortcut_capture_stop()`, and receives `shortcut-capture` events with `mods`, `code`, `key`, the formatted keycaps and a `done` flag, so each key appears as it is pressed.

While capture is on:

- The dictation loop drops hotkey events, so fn starts nothing. Capture refuses to start while a dictation is listening.
- On macOS the swallowing tap withholds and reports every key event, and the seven menu accelerators are removed and later restored. ⌘⇧R, ⌘N, ⌘F and ⌘1 then reach the recorder and nothing else. fn and sides come from the tap's flags, so the recorder does not depend on WebKit reporting a bare fn, which is unverified.
- On Windows the hook withholds and reports every key, and the plugin registrations are released for the duration.
- On X11 the grabs are released and XRecord reports the keys. The keydown handler ignores commands while capture is on.
- On Wayland, and on a Mac without Accessibility, there is no native capture. The in-app rows record from DOM keydown, and the global rows are disabled anyway.

Capture ends with a binding when a key goes down, or when all modifiers are released with no key (a modifier-only binding with sides). Esc alone cancels, so a bare Esc comes back only through reset. Losing window focus or removing the row also cancels.

### FFI between Rust and Swift

- `pennel_mac_hotkey_start(bindings_json, sink, ctx)` replaces the trigger string. The JSON has `hold`, `always` and `listening` lists with numeric keycodes and modifier masks. Rust owns the table from W3C code names to macOS keycodes, so Swift stays a matcher.
- `pennel_mac_hotkey_set_bindings(handle, bindings_json) -> Bool` replaces `_set_trigger` and `_set_record_shortcut_enabled`.
- `pennel_mac_hotkey_set_listening(handle, Bool)` switches the `listening` list on and off.
- `pennel_mac_hotkey_capture(handle, text_sink, ctx) -> Bool` starts capture with a text sink that receives one JSON line per key event (keycode, flags, typed character). A null sink stops it.
- `_held` and `_stop` stay.
- The sink's third argument widens from `Bool` to `UInt32`. Codes 0 down, 1 up, 2 interrupt and 3 shift keep their meaning. Code 4 becomes "action fired", with the action number as value: 1 record, 2 open, 3 cancel, 4 next model.

In Rust, `NativeHotkeyEvent::MeetingShortcut` becomes `Action(u32)`, and `Hotkey::set_trigger` and `set_record_shortcut_enabled` become `set_bindings`.

### One label source

Core formats a binding for a platform and interface language into keycaps (`["⌃", "⌥", "A"]`, `["right ⌥"]`, `["Ctrl", "Shift", "R"]`) and one text form. The shell puts both in `AppInfo.shortcuts` and in capture events. TypeScript renders them and never builds a label. This removes `hotkeyLabel` and `hotkeyKeyLabel` (`presentation.ts`), `hotkey_label` (`main.rs`) and the literals in `views/settings.ts:226`, `record_shortcut.rs:70` and `macos_tray.rs`.

Everything that shows a shortcut reads the row for its action: the home keycap and the pad (`views/pad.ts`), onboarding and practice (`views/onboarding.ts`), the Meetings setting, the tray hint and tray key equivalents, the Mac app menu, the search hint, the pill hints, the recording-shortcut error text and the seven locale strings that take `{shortcut}`. With Dictate cleared, the home reads "Dictate with the button." and those strings need a variant without a key. The tray's "Open Pennel" item shows the Open Pennel binding, so with the proposed default it shows no key.

## Consequences

- An older build always starts, and a build with this field keeps ids and tokens it does not know. A downgrade loses custom bindings.
- Each binding carries two key names, because the tap matches by position while labels and menus match by character.
- Esc is withheld from the front app while the pill listens. It still reaches the app at any other time.
- X11 cannot bind Dictate to a key in this version, and Wayland shows every global row disabled. Both go in `docs/desktop.md`.
- `DESIGN.md:11` gains a sixth Settings category in T10g, and the capability table in `docs/desktop.md` changes in T10d and T10e.
- `Settings` is a uniffi record, so the mobile bindings regenerate and the mobile checks run. The hand-written mirror in `apps/mobile/modules/pennel-mobile/src/types.ts` needs no edit: the field is not serialised when empty, and `patch_mobile_settings` rejects keys the current settings do not have.
- No existing ADR or glossary term changes.
- These need a device and stay open after the tests: fn in the recorder, withholding in other apps, digits behind Shift on AZERTY, secure input, and everything on Windows and Linux beyond one manual CI run.

## Alternatives considered

- New values on `HotkeyTrigger`, or any enum inside `shortcuts`. An older build would fail to start on the first unknown value, and the uniffi enum would change for mobile.
- One key name. Characters alone break a Mac global shortcut whenever a non-Latin layout is active, and the tap reports positions. Positions alone mislabel keys on AZERTY and do not fit the menu or the keydown handler, which match characters.
- A registration API for one-press combinations on macOS. It cannot see fn, sides or a bare modifier, so the tap would remain beside it as a second mechanism.
- Raw hooks for everything on Windows and X11, without the plugin. XRecord cannot withhold a key, and on Windows registration reports when another app holds the keys.
- A recorder built on DOM keydown. The tap swallows ⌘⇧R before the page sees it, the Mac menu takes ⌘N, and a bare fn probably never arrives.
- Storing every action's binding. That freezes today's defaults into each file and makes "differs from the default" depend on stored data.
- Sending every macOS key event to the Rust tracker, for one matcher. Each keystroke would cross the FFI inside an active tap, and a working Swift service would be rewritten. A shared fixture keeps the two matchers aligned.

## Migration and downgrade

There is no migration step. A file without `shortcuts` behaves exactly as today, because every binding derives from the old fields and the defaults.

An older build ignores `shortcuts` on load and starts, because `hotkey` always holds one of the five existing values. On its next save it drops the field. Custom bindings are then gone. Dictate falls back to the preset in `hotkey`, or to the platform key if the binding was custom or cleared, since the enum has no value for "none". Record follows the flag: on with ⌘⇧R when it was at its default, off when it was custom or cleared. Upgrading again starts from those values.

## Test plan

Rust, core: lenient parsing (wrong types, unknown ids and tokens, round trip, `load_strict` never fails on `shortcuts`); `hotkey` always serialises as one of five values; derivation for each preset on each platform; write-back and the legacy-write guard; each validation rule; conflicts across scopes and modifier containment; "Use it here" in one save; reset and reset all; the capability table for four platforms; labels per platform in English and Dutch; `cancel()` from every phase, with the existing gesture tests unchanged.

Rust, desktop: tracker edges for a binding with a key; own key and repeats raise no interrupt; withholding latched until key-up; listening keys withheld only while listening; the old constructor equals the new binding for all five presets; the code-name table; the bindings JSON; menu accelerators from the table.

Swift: the matching moves into a plain type, tested like `EventTapHealth` with synthetic events: hold with a key, the latch, the listening list, capture, sides and fn. One JSON fixture of key sequences runs through both the Rust tracker and the Swift matcher.

TypeScript: `desktopShortcut` from the table, with today's cases passing on defaults; the recorder states (pressing, done, conflict, cleared, cancelled); interaction checks for record, Esc, clear, reset, reset all and "Use it here"; English and Dutch parity; renders of screens 26 to 29 in light and dark at 1280×860 and 760×520.

Elsewhere: the mobile type and unit checks, and one `desktop.yml` run with `platforms=all` for T10e.

## Pull requests

The plan's seven PRs and their order hold, with a correction to six of them. Until T10f nothing writes `shortcuts`, so code that reads effective bindings and code that still reads the old fields always agree.

| ID | Scope | Correction |
| --- | --- | --- |
| T10a | Core: shape, lenient parsing, derivation, the write function with write-back and guard, validation, conflicts, capability table, labels | Labels and the capability table move here from T10f and T10e, because they are pure core and later PRs use them. The mobile mirror is not edited |
| T10b | `cancel()` in the gesture; tracker with key codes, hold bindings and the withhold decision | The withhold decision lives here, where routine CI compiles it. The old constructor stays, so Windows and Linux files are untouched |
| T10c | In-app table for the Mac menu and the keydown handler, beside T10b | None |
| T10d | macOS: bindings FFI, tap table, hold with a key, Record and Open Pennel, capture | Cancel is built but left out of the `listening` list, so Esc does not behave differently on one platform |
| T10e | Windows and Linux glue, X11 grabs, Wayland reasons, capture, the "esc cancel" hint | Cancel is switched on for every platform here, with the hint. One `platforms=all` run |
| T10f | Recorder, Shortcut row, every display reads the row | The preset buttons and the Meetings toggle move to the core function in this PR, since it adds the first writer of `shortcuts` |
| T10g | Shortcuts page | The Next speech model row is disabled with "Set up a second speech model first" until T11b |

T11b then only wires the next-model action and enables its row. It needs no hook work, because the `listening` list already routes the key.

## Questions for the owner

1. Open Pennel default. The design shows ⌘O, which today is only the key equivalent inside the open tray menu. As a global shortcut it would take Open away from every other app. Recommendation: None, and the tray item shows no key until one is set.
2. Next speech model default. The design shows ⌥, which collides with the Right Option preset, and a bare Alt opens menus on Windows. Recommendation: Left Option on macOS and None on Windows and Linux until that is checked on those systems.
3. Dictate with an ordinary key on X11. XRecord cannot withhold the key, and a grab with press and release cannot be checked while Linux validation is paused. Recommendation: refuse it on X11 in this version with a reason, and keep modifier-only bindings there.
4. Downgrade values. Recommendation: a custom Record binding writes `record_shortcut_enabled: false`, so an older build leaves ⌘⇧R alone, and a custom or cleared Dictate writes the platform key to `hotkey`, since an older build must have one.
5. What Cancel covers. Recommendation: it cancels while the pill listens, in a hold and in hands-free, and does not stop a dictation that is already being processed. Stopping queued work touches the paste queue and can follow later.
6. The Meetings toggle "Use ⌘⇧R to start or stop recording". Recommendation: keep the row, show the real keys, let off clear the shortcut and on restore the default, and add a link to the Shortcuts page.

## T10e implementation safety gap

The table above is the accepted target. The pinned X11 registration dependency cannot acknowledge a completed ungrab or recover the original grabbed keycode after a keyboard-map change. The implementation therefore disables X11 keyed globals in this version, with a localized reason, instead of installing a grab that might remain outside its scope. Modifier-only dictation/listening, in-app shortcuts and the existing button/tray/CLI paths remain. Re-enabling keyed X11 shortcuts requires an owned, tested release boundary; no new dependency was introduced. See the implementation gap in `docs/desktop.md`. The accepted Windows/X11 layout-named `code` contract and per-machine storage remain unchanged.
