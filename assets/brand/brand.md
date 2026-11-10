# Pennel brand

Pennel (PEN-el, the same in Dutch and English): the little pen that writes down what you say. Dictation where you type, and meeting notes kept on your computer. One Rust core serves macOS, Windows and Linux.

## Identity

Use the checked-in `wordmark.svg`, `mark.svg`, `app-icon.svg` and `menubar.svg`. The wordmark uses Instrument Serif and a true circular orange dot. The listening indicator uses five bars; movement conveys actual audio level or processing state.

The desktop interface follows the user-approved [Glaze direction #92](https://github.com/pennel-app/pennel/issues/92): neutral surfaces, locally bundled Inter, compact titles, inset rounded panes, capsule controls and real system window chrome. Pennel retains its marks and copy. The mobile app keeps the warm paper/ink palette, Figtree UI and Instrument Serif large titles. The shared Tauri app owns desktop product UI; the retired Swift shell at `91775c1` remains a historical behavior reference.

## Palette and source of truth

[`tokens.json`](tokens.json) owns semantic colors, spacing, radii and type sizes. Base colors and typography serve mobile, artwork and the fixed dictation pill. The `desktop-theme` group overrides the main desktop frontend only and generates `apps/desktop/src/desktop-theme.generated.css`; it is not imported by the separate pill entrypoint. Run `python3 scripts/generate-theme.py` from the repository root after editing tokens. Generated CSS, pill metrics and shared SVG icons are checked in; `--check` verifies their contents and text contrast, and `--self-test` exercises the generator. Retired shell Swift outputs are no longer generated.

Desktop palette:

| Role | Light | Dark |
|---|---|---|
| Content | `#FFFFFF` | `#101010` |
| Sidebar | `#EFEFEF` | `#161616` |
| Foreground | `#171717` | `#F2F2F2` |
| Primary button | `#006FDF` | `#006FDF` |
| Primary button text | `#FFFFFF` | `#FFFFFF` |
| Live (recording signal) | `#C2410C` | `#F2B48E` |

Navigation selection and ordinary toolbar actions stay neutral. Blue marks primary submission and focus; errors and destructive actions have a separate red role. Native glass has contrast-protecting navigation and toolbar backings. All secondary text must remain readable on its actual surface.

Base mobile palette:

| Role | Light | Dark |
|---|---|---|
| Surface | `#FCFBF8` | `#1E1C1A` |
| Window chrome | `#FCFBF8` | `#1E1C1A` |
| Sidebar | `#F7F4EE` | `#252320` |
| Foreground | `#2A2723` | `#F3F0EA` |
| Secondary text | `#6F675D` | `#ABA398` |
| Border | `#ECE7DE` | `#3A3632` |
| Selected surface | `#ECE7DE` | `#3A3632` |
| Field | `#FFFFFF` | `#2C2926` |
| Primary button | `#2A2723` | `#F3F0EA` |
| Primary button text | `#FCFBF8` | `#1E1C1A` |
| Accent | `#C2410C` | `#F2B48E` |
| Success | `#1E7B3A` | `#7BD88F` |

The artwork's Paper `#FCFBF8`, Ink `#2A2723` and Soft wave `#F2B48E` remain fixed brand colors. The dictation pill uses its own fixed Ink background, Paper text and Soft wave indicator in both appearances. Secondary text, including selected-row metadata, must retain at least 4.5:1 contrast.

Legacy UI names map to semantic roles: `paper` means Surface, `background` means Subtle, `ink` means Foreground and `line` means Border. They adapt with appearance; fixed artwork and pill colors use their dedicated roles.

Rules
- **One accent per surface.** Desktop uses blue for primary actions/focus and neutral navigation selection; keep error/destructive roles separate. The mobile base palette uses orange for active state, focus or an actionable error and neutral primary buttons. Success green confirms a finished step with text.
- **Orange never sits on ink.** Fixed dark artwork and the dictation pill use Soft wave. Mobile dark appearance follows the base palette's readable accent.
- **No gradients, no shadows in marks, no glows.** Icons are flat.
- The dot is always a true circle, never a font period.

## Typography and geometry

- **Inter** carries desktop navigation, body text, controls, labels and compact headings. The chrome font role follows the desktop interface font. Use Regular for prose, Medium for controls and SemiBold for short headings. Desktop UI text is about 13 px, captions start at 11 px, page titles are 26 px and the dictation instruction is 30 px.
- **Figtree** carries mobile navigation, body text, buttons, labels and small section headings. **Instrument Serif** Regular carries mobile large titles, the unchanged wordmark artwork and marketing. Never fake bold or use serif type in controls. Desktop app headings use Inter.
- Use sentence case. Base mobile sizes and geometry remain defined by the existing base tokens; desktop overrides must not change them. Enlarge text without losing critical content or actions.
- Desktop uses 6 px outer insets, 16 px pane corners, 24 px dialogs, capsule controls and compact source lists. The toolbar band is 52 px with 36 px controls. Ordinary controls are 32 px high, compact controls 28 px, sidebar search 30 px and navigation rows at least 34 px. The sidebar defaults to 220 px, preserves saved widths within 200–320 px and remains usable at a 760×520 workspace minimum.
- Keep one sidebar search field and restore it with Cmd/Ctrl+F when navigation is collapsed. The quiet sidebar toggle sits above its trailing edge, the header context aligns with the reading column, and New recording/Return to recording is vertically centered and aligned with the footer action edge at a 22 px content inset.
- Keep component state families consistent: default, hover, focus, selected, disabled, loading and error. A disabled primary button keeps a readable label; hover does not replace its background with a pale secondary fill.
- macOS navigation and toolbar material is system-rendered Liquid Glass on macOS 26+, with legacy visual-effect and opaque accessibility fallbacks. Reading content stays on the semantic opaque surface. Native material availability never changes the shared labels, actions or document hierarchy.
- Base fonts live in `assets/brand/fonts/`; desktop Inter and the original Forma styles live in `apps/desktop/src/desktop-ui/`. Bundle fonts locally and retain both font and style licenses. Outline artwork text so SVG marks render without installed fonts.

## Writing the name

- In running text it is always **Pennel**: capital P, one word, double n, single l. Never "PENNEL", "PenNel", "Pennell" or "Pennel app".
- The lowercase **pennel.** (with the full stop) exists only as the wordmark artwork. Don't type it in sentences.
- Verb use is fine in casual copy: "Just pennel it." / "Even pennellen." Don't overdo it; once per page at most.
- Repo, CLI and URLs: `pennel` (lowercase). Existing package configuration owns bundle identifiers and their preservation across upgrades.

## Naming system

| Name | What it is |
|---|---|
| **Pennel** | The open-source desktop product (Apache-2.0). Local models or your chosen provider. |
| **Pennel Cloud** | Reserved name for a hosted personal service. Availability and features require separate release evidence. |
| **Pennel for Teams** | Reserved name for organisation services. Availability and features require separate release evidence. |

- Avoid "Pennel Pro" or "Plus": it suggests the open-source app is crippled, which hurts trust in an open-core product.
- Feature names are plain nouns in sentence case, written natively per language:

| English | Dutch |
|---|---|
| Dictation | Dicteren |
| Meetings | Vergaderingen |
| Notes | Notities |
| Ask | Vraag |
| Speakers | Sprekers |
| Search | Zoeken |

## Voice and tone

Calm, direct, a bit dry: the way a good colleague writes a meeting recap. Confident about privacy without preaching. Belgian modesty: show, don't shout. Dutch and English are equals; write each one natively, never word for word.

**Do**
- Short sentences, verbs first. "Hold fn. Talk. Let go." / "Hou fn in. Praat. Laat los."
- True, concrete facts: "no account", "on your computer", "Markdown files in a folder you choose". Size, speed and accuracy figures require a dated measurement of the stated build and configuration.
- Say where data lives and who sees it.
- Use the reader's words: emails, notes, meetings. Not "AI workflows".
- Dutch: use "je", not "u". Keep English UI terms only where Dutch users really use them (Mac, fn, Markdown).

**Don't**
- No "AI-powered", "revolutionary", "supercharge", "10x", "magic", sparkles, robots, microphone emoji or gradients.
- No perfection claims. "Cleans up", not "flawless".
- No fear-selling about privacy. State the facts and let them land.
- Don't pun on "pen" more than once per page.

**Examples**

| | English | Dutch |
|---|---|---|
| Onboarding | Hold fn and say something. Pennel writes it where your cursor is. | Hou fn in en zeg iets. Pennel schrijft het waar je cursor staat. |
| Privacy | Your recordings stay on your computer. Nothing leaves unless you choose a provider. | Je opnames blijven op je computer. Er gaat niets weg, tenzij je zelf een provider kiest. |
| Error | Couldn't reach your provider. Your recording is saved; try again when you're back online. | Je provider is niet bereikbaar. Je opname is bewaard; probeer opnieuw als je weer online bent. |

## Taglines

| # | English | Dutch |
|---|---|---|
| 1 | *Say it. Full stop.* (lead line) | *Zeg het. Punt uit.* (lead line) |
| 2 | *Speak. It's written.* | *Spreek. Het staat er.* |
| 3 | *Talk in. Text out.* | *Praat erin. Tekst eruit.* |
| 4 | *Your voice, in ink.* | *Je stem, in inkt.* |
| 5 | *Your meetings, written down. On your Mac.* | *Je vergaderingen, uitgeschreven. Op je Mac.* |
| 6 | *Written the way you meant it.* | *Geschreven zoals je het bedoelde.* |

The Dutch lines are written natively, not translated; they don't have to match the English word for word.

## Files

- `tokens.json`: canonical base roles/dimensions and the desktop-only `desktop-theme` overrides.
- `fonts/`: bundled Figtree and Instrument Serif files and licenses.
- `mark.svg`, `wordmark.svg`, `app-icon.svg`, `menubar.svg`: existing flat identity artwork.
- `scripts/generate-theme.py`: dependency-free CSS/pill-metric/icon generator and checks.
- `apps/desktop/src/desktop-ui/`: original Forma CSS, bundled desktop Inter and licenses.
- `assets/ui/icons/manifest.json`: compact interface glyph source, emitted as local SVGs and native template-image data.
- `DESIGN.md` at repository root: product composition, interaction and appearance rules.
