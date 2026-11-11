#!/usr/bin/env python3
"""Generate shared desktop theme, pill metrics and SVG icons from Pennel's design tokens."""

from __future__ import annotations

import argparse
import copy
from html import escape
import json
import math
from pathlib import Path
import re
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
TOKENS = ROOT / "assets/brand/tokens.json"
CSS = ROOT / "apps/desktop/src/theme.generated.css"
DESKTOP_CSS = ROOT / "apps/desktop/src/desktop-theme.generated.css"
PILL_METRICS = ROOT / "apps/desktop/src/pill-metrics.generated.ts"
ICON_DIRECTORY = ROOT / "assets/ui/icons"
ICON_MANIFEST = ICON_DIRECTORY / "manifest.json"
TOKEN_NAME = re.compile(r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$")
HEX_COLOR = re.compile(r"^#[0-9a-fA-F]{6}(?:[0-9a-fA-F]{2})?$")
PILL_COUNTS = {"preview-lines", "preview-limit"}
CONTRAST_PAIRS = (
    ("foreground", "surface"),
    ("foreground", "sidebar"),
    ("foreground", "selection"),
    ("muted", "surface"),
    ("muted", "subtle"),
    ("muted", "field"),
    ("muted", "sidebar"),
    ("muted", "selection"),
    ("button-text", "button"),
    ("accent", "surface"),
    ("success", "surface"),
    ("success", "subtle"),
    ("disabled-text", "disabled"),
    ("pill-text", "pill-background"),
    ("pill-accent", "pill-background"),
    *((f"voice-{number}", "surface") for number in range(1, 5)),
)


def channels(value: str) -> tuple[float, float, float]:
    return tuple(int(value[index : index + 2], 16) / 255 for index in (1, 3, 5))


def luminance(value: str) -> float:
    linear = tuple(channel / 12.92 if channel <= 0.04045 else ((channel + 0.055) / 1.055) ** 2.4 for channel in channels(value))
    return sum(channel * coefficient for channel, coefficient in zip(linear, (0.2126, 0.7152, 0.0722)))


def contrast(foreground: str, background: str) -> float:
    first, second = sorted((luminance(foreground), luminance(background)), reverse=True)
    return (first + 0.05) / (second + 0.05)


def composite(foreground: str, background: str) -> str:
    alpha = int(foreground[7:9], 16) / 255 if len(foreground) == 9 else 1
    blended = [round((front * alpha + back * (1 - alpha)) * 255)
               for front, back in zip(channels(foreground), channels(background))]
    return "#" + "".join(f"{channel:02X}" for channel in blended)


def validate(tokens: dict) -> None:
    if tokens.get("schema") != 1:
        raise ValueError("unsupported theme token schema")
    colors = tokens.get("colors", {})
    required = {name for pair in CONTRAST_PAIRS for name in pair}
    required.update(("control-edge", "chrome", "navigation-backing", "navigation-muted", "toolbar-control", "toolbar-control-hover", "toolbar-control-highlight", "border", "hover", "focus-ring", "shadow", "key-shadow", "brand-paper", "brand-ink", "wave"))
    if missing := required - colors.keys():
        raise ValueError(f"missing semantic colors: {', '.join(sorted(missing))}")
    for name, pair in colors.items():
        if not TOKEN_NAME.fullmatch(name) or set(pair) != {"light", "dark"}:
            raise ValueError(f"invalid color token: {name}")
        for value in pair.values():
            if not isinstance(value, str) or not HEX_COLOR.fullmatch(value):
                raise ValueError(f"invalid hex color for {name}: {value}")
    for name, target in tokens.get("aliases", {}).items():
        if not TOKEN_NAME.fullmatch(name) or name in colors or target not in colors:
            raise ValueError(f"invalid theme alias: {name}")
    for foreground, background in CONTRAST_PAIRS:
        for appearance in ("light", "dark"):
            ratio = contrast(colors[foreground][appearance], colors[background][appearance])
            if ratio < 4.5:
                raise ValueError(f"{appearance}: {foreground} on {background} has {ratio:.2f}:1 contrast; expected 4.5:1")
    for name in ("pill-background", "pill-text", "pill-accent"):
        if colors[name]["light"].lower() != colors[name]["dark"].lower():
            raise ValueError(f"{name} must remain stable across appearances")
    for appearance in ("light", "dark"):
        for backdrop in ("#000000", "#FFFFFF"):
            backing = composite(colors["navigation-backing"][appearance], backdrop)
            if any(contrast(colors[role][appearance], backing) < 4.5 for role in ("foreground", "navigation-muted")):
                raise ValueError(f"{appearance}: navigation text must retain 4.5:1 contrast over either backdrop extreme")
            for role in ("toolbar-control", "toolbar-control-hover"):
                control = composite(colors[role][appearance], backing)
                if contrast(colors["foreground"][appearance], control) < 4.5:
                    raise ValueError(f"{appearance}: toolbar control labels must retain 4.5:1 contrast over either backdrop extreme")
    for section in ("spacing", "radii", "desktop"):
        for name, value in tokens[section].items():
            if not (str(name).isdigit() or TOKEN_NAME.fullmatch(name)):
                raise ValueError(f"invalid {section} token name: {name}")
            if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
                raise ValueError(f"invalid {section} token: {name}")
    for name, value in tokens["pill"].items():
        if not TOKEN_NAME.fullmatch(name) or isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
            raise ValueError(f"invalid pill metric: {name}")
        if name in PILL_COUNTS and not isinstance(value, int):
            raise ValueError(f"invalid pill count: {name}")
    themes = tokens.get("pill-themes", {})
    if themes:
        if set(themes) != {"ink", "light"}:
            raise ValueError("invalid pill themes")
        for name, theme in themes.items():
            if set(theme) != {"background", "text", "accent"} or any(not isinstance(value, str) or not HEX_COLOR.fullmatch(value) for value in theme.values()):
                raise ValueError(f"invalid pill theme: {name}")
            if any(contrast(theme[role], theme["background"]) < 4.5 for role in ("text", "accent")):
                raise ValueError(f"pill {name} contrast must be at least 4.5:1")
    sizes = tokens.get("pill-sizes", {})
    if sizes:
        if set(sizes) != {"compact", "regular", "large"}:
            raise ValueError("invalid pill size variants")
        for name, metrics in sizes.items():
            if set(metrics) != set(tokens["pill"]):
                raise ValueError(f"invalid pill metrics for {name}")
            for metric, value in metrics.items():
                if (isinstance(value, bool) or not isinstance(value, (int, float))
                        or not math.isfinite(value) or value <= 0
                        or metric in PILL_COUNTS and not isinstance(value, int)
                        or metric in {"label-size", "caption-size", "content-size"} and not 11 <= value <= 48
                        or metric == "preview-lines" and value > 2):
                    raise ValueError(f"invalid pill metric {name}: {metric}")
            # Caption plus a two-line title/detail must fit without clipping a recovery message.
            required = (2 * metrics["outer-inset"] + 2 * tokens["spacing"]["3"] + tokens["spacing"]["2"]
                        + metrics["preview-lines"] * metrics["preview-line-height"]
                        + math.ceil(4 * metrics.get("label-size", tokens["typography"]["sizes"]["label"]) * 1.2) + 1)
            if metrics["window-height"] < required:
                raise ValueError(f"pill {name} frame cannot fit its bounded caption and recovery message")
        if sizes["regular"] != tokens["pill"]:
            raise ValueError("regular pill metrics must retain the shared defaults")
    typography = tokens["typography"]
    if typography["ui"] != "Figtree" or typography["display"] != "Instrument Serif":
        raise ValueError("the bundled Pennel font families must be retained")
    if typography.get("interface", "brand") not in ("brand", "system"):
        raise ValueError("typography interface must be brand or system")
    if typography.get("macos-chrome", "brand") not in ("brand", "system"):
        raise ValueError("macOS chrome typography must be brand or system")
    if not tokens["desktop"]["sidebar-min-width"] <= tokens["desktop"]["sidebar-width"] <= tokens["desktop"]["sidebar-max-width"]:
        raise ValueError("desktop sidebar width must be within its bounds")
    for name, value in typography["sizes"].items():
        if not TOKEN_NAME.fullmatch(name) or not isinstance(value, (int, float)) or not 11 <= value <= 48:
            raise ValueError(f"invalid typography size: {name}")
    if "desktop-theme" in tokens:
        validate_desktop_theme(tokens)


def validate_desktop_theme(tokens: dict) -> None:
    desktop = tokens["desktop-theme"]
    if not isinstance(desktop, dict) or set(desktop) != {"colors", "aliases", "radii", "typography"}:
        raise ValueError("invalid desktop theme sections")
    for section in ("colors", "aliases", "radii", "typography"):
        if not isinstance(desktop[section], dict):
            raise ValueError(f"invalid desktop theme {section}")
    if any(name.startswith(("pill-", "brand-", "voice-")) or name == "wave" for name in desktop["colors"]):
        raise ValueError("desktop theme must retain fixed brand and pill colors")
    fixed = {name for name in tokens["colors"] if name.startswith(("pill-", "brand-", "voice-")) or name == "wave"}
    required = (tokens["colors"].keys() - fixed) | {"border-strong", "control-surface", "surface-raised", "accent-hover", "accent-soft", "success-soft", "danger", "danger-soft", "live"}
    if missing := required - desktop["colors"].keys():
        raise ValueError(f"missing desktop semantic colors: {', '.join(sorted(missing))}")
    if not {"control", "segmented", "panel", "window", "dialog"} <= desktop["radii"].keys():
        raise ValueError("missing desktop radius roles")
    if desktop["aliases"] != {"error": "danger"}:
        raise ValueError("desktop errors must use the danger role")
    typography = desktop["typography"]
    if set(typography) != {"ui", "display", "sizes"} or typography["ui"] != "Desktop Inter" or typography["display"] != "Desktop Inter":
        raise ValueError("the desktop theme must use the bundled Desktop Inter family")
    if not isinstance(typography["sizes"], dict) or not typography["sizes"].keys() <= tokens["typography"]["sizes"].keys():
        raise ValueError("invalid desktop typography sizes")
    merged = copy.deepcopy(tokens)
    del merged["desktop-theme"]
    for section in ("colors", "aliases", "radii"):
        merged[section].update(desktop[section])
    merged["typography"]["sizes"].update(typography["sizes"])
    validate(merged)
    for foreground, background in (("danger", "surface"), ("danger", "subtle"), ("danger", "danger-soft"), ("success", "success-soft"), ("accent", "accent-soft"), ("foreground", "control-surface"), ("muted", "control-surface"), ("foreground", "surface-raised"), ("muted", "surface-raised"),
                                   ("live", "surface"), ("live", "subtle"), *((f"voice-{number}", background) for number in range(1, 5) for background in ("surface", "subtle"))):
        for appearance in ("light", "dark"):
            ratio = contrast(merged["colors"][foreground][appearance], merged["colors"][background][appearance])
            if ratio < 4.5:
                raise ValueError(f"desktop {appearance}: {foreground} on {background} has {ratio:.2f}:1 contrast; expected 4.5:1")


def camel_name(name: str) -> str:
    words = name.split("-")
    return words[0] + "".join(word[:1].upper() + word[1:] for word in words[1:])


def scalar(value: int | float) -> str:
    return f"{value:g}"


def render_css(tokens: dict) -> str:
    lines = ["/* Generated by scripts/generate-theme.py from assets/brand/tokens.json. */", ":root {", "  color-scheme: light;"]
    for name, pair in tokens["colors"].items():
        lines.append(f"  --{name}: {pair['light'].lower()};")
    for name, target in tokens["aliases"].items():
        lines.append(f"  --{name}: var(--{target});")
    for name, value in tokens["spacing"].items():
        lines.append(f"  --space-{name}: {scalar(value)}px;")
    for name, value in tokens["radii"].items():
        lines.append(f"  --radius-{name}: {scalar(value)}px;")
    for name, value in tokens["desktop"].items():
        lines.append(f"  --desktop-{name}: {scalar(value)}px;")
    for name, value in tokens["pill"].items():
        unit = "" if name in PILL_COUNTS else "px"
        lines.append(f"  --pill-{name}: {scalar(value)}{unit};")
    brand_ui = f"\"{tokens['typography']['ui']}\", -apple-system, BlinkMacSystemFont, \"Segoe UI\", system-ui, sans-serif"
    # "system" follows each platform's own UI font (SF, Segoe UI, Cantarell/Ubuntu); "brand" is Figtree, as in the original app.
    system_ui = "-apple-system, BlinkMacSystemFont, \"Segoe UI\", system-ui, Cantarell, Ubuntu, \"Noto Sans\", sans-serif"
    interface = system_ui if tokens["typography"].get("interface") == "system" else brand_ui
    lines.extend((
        f"  --ui: {interface};",
        "  --chrome-ui: var(--ui);",
        f"  --brand-ui: {brand_ui};",
        f"  --serif: \"{tokens['typography']['display']}\", Georgia, serif;",
    ))
    for name, value in tokens["typography"]["sizes"].items():
        lines.append(f"  --font-{name}: {scalar(value)}px;")
    lines.extend(("}", "", "/* System appearance uses a media query supported by the minimum WebKit. */", "@media (prefers-color-scheme: dark) {", "  :root {", "    color-scheme: dark;"))
    for name, pair in tokens["colors"].items():
        lines.append(f"    --{name}: {pair['dark'].lower()};")
    lines.extend(("  }", "}", ""))
    for appearance in ("light", "dark"):
        lines.extend((f':root[data-appearance="{appearance}"], :root[data-theme="{appearance}"] {{', f"  color-scheme: {appearance};"))
        for name, pair in tokens["colors"].items():
            lines.append(f"  --{name}: {pair[appearance].lower()};")
        lines.extend(("}", ""))
    if tokens["typography"].get("macos-chrome") == "system":
        lines.extend((':root[data-platform="macos"] {', f"  --chrome-ui: {system_ui};", "}", ""))
    if tokens.get("pill-themes") or tokens.get("pill-sizes"):
        lines.append("/* Configurable pill variants; scoped to the pill and its settings preview. */")
    def selector(attribute: str, value: str) -> str:
        return f':root[data-pill-{attribute}="{value}"], [data-pill-preview][data-pill-{attribute}="{value}"]'
    for name, theme in tokens.get("pill-themes", {}).items():
        lines.append(selector("colour", name) + " {")
        lines.extend(f"  --pill-{role}: {value.lower()};" for role, value in theme.items())
        lines.extend(("}", ""))
    if tokens.get("pill-themes"):
        for dark, theme in ((False, tokens["pill-themes"]["light"]), (True, tokens["pill-themes"]["ink"])):
            if dark: lines.append("@media (prefers-color-scheme: dark) {")
            lines.append(selector("colour", "system") + " {")
            lines.extend(f"  --pill-{role}: {value.lower()};" for role, value in theme.items())
            lines.append("}")
            if dark: lines.append("}")
            lines.append("")
    if tokens.get("pill-themes"):
        for appearance, theme in (("light", tokens["pill-themes"]["light"]), ("dark", tokens["pill-themes"]["ink"])):
            lines.append(f':root[data-pill-colour="system"][data-pill-appearance="{appearance}"], [data-pill-preview][data-pill-colour="system"][data-pill-appearance="{appearance}"] {{')
            lines.extend(f"  --pill-{role}: {value.lower()};" for role, value in theme.items())
            lines.extend(("}", ""))
    for name, metrics in tokens.get("pill-sizes", {}).items():
        lines.append(selector("size", name) + " {")
        lines.extend(f"  --pill-{metric}: {scalar(value)}{'' if metric in PILL_COUNTS else 'px'};" for metric, value in metrics.items())
        lines.extend(("}", ""))
    return "\n".join(lines)


def render_pill_metrics(tokens: dict) -> str:
    lines = ["// Generated by scripts/generate-theme.py from assets/brand/tokens.json.", "export const PILL_METRICS = {"]
    for name, value in tokens["pill"].items():
        lines.append(f"  {camel_name(name)}: {scalar(value)},")
    lines.extend(("} as const;", ""))
    lines.append("export const PILL_SIZES = {")
    for name, metrics in tokens.get("pill-sizes", {"regular": tokens["pill"]}).items():
        lines.append(f"  {name}: {{")
        lines.extend(f"    {camel_name(metric)}: {scalar(value)}," for metric, value in metrics.items())
        lines.append("  },")
    lines.extend(("} as const;", ""))
    return "\n".join(lines)


def render_desktop_css(tokens: dict) -> str:
    desktop = tokens["desktop-theme"]
    lines = ["/* Generated desktop-only overrides; the pill and mobile retain the base brand theme. */", ":root {", "  color-scheme: light;"]
    for name, pair in desktop["colors"].items():
        lines.append(f"  --{name}: {pair['light'].lower()};")
    for name, target in desktop["aliases"].items():
        lines.append(f"  --{name}: var(--{target});")
    for name, value in desktop["radii"].items():
        lines.append(f"  --radius-{name}: {scalar(value)}px;")
    lines.extend((
        f"  --ui: \"{desktop['typography']['ui']}\", -apple-system, BlinkMacSystemFont, \"Segoe UI\", system-ui, sans-serif;",
        "  --chrome-ui: var(--ui);",
        "  --serif: var(--ui);",
    ))
    for name, value in desktop["typography"]["sizes"].items():
        lines.append(f"  --font-{name}: {scalar(value)}px;")
    lines.extend(("}", "", "@media (prefers-color-scheme: dark) {", "  :root {", "    color-scheme: dark;"))
    for name, pair in desktop["colors"].items():
        lines.append(f"    --{name}: {pair['dark'].lower()};")
    lines.extend(("  }", "}", ""))
    for appearance in ("light", "dark"):
        lines.extend((f':root[data-appearance="{appearance}"], :root[data-theme="{appearance}"] {{', f"  color-scheme: {appearance};"))
        for name, pair in desktop["colors"].items():
            lines.append(f"  --{name}: {pair[appearance].lower()};")
        lines.extend(("}", ""))
    bridge = {
        "font": "var(--ui)",
        "bg": "var(--surface)",
        "sidebar-bg": "var(--sidebar)",
        "surface": "var(--surface)",
        "surface-raised": "var(--surface-raised)",
        "surface-inset": "var(--subtle)",
        "text": "var(--foreground)",
        "text-muted": "var(--muted)",
        "text-faint": "var(--muted)",
        "border": "var(--border)",
        "border-strong": "var(--border-strong)",
        "accent": "var(--button)",
        "on-accent": "var(--button-text)",
        "accent-hover": "var(--accent-hover)",
        "accent-soft": "var(--accent-soft)",
        "accent-text": "var(--accent)",
        "hover": "var(--hover)",
        "selected": "var(--selection)",
        "danger": "var(--danger)",
        "danger-soft": "var(--danger-soft)",
        "success": "var(--success)",
        "success-soft": "var(--success-soft)",
        "focus": "var(--focus-ring)",
        "shadow": "0 1px 2px var(--shadow)",
        "dialog-shadow": "0 24px 80px var(--shadow)",
        "radius": "var(--radius-control)",
        "pane-radius": "var(--radius-panel)",
        "toolbar-height": "var(--desktop-toolbar-height)",
        "sidebar-inset": "var(--desktop-shell-inset)",
        "control-sm": "var(--desktop-compact-control-height)",
        "control-md": "var(--desktop-control-height)",
        "control-toolbar": "var(--desktop-toolbar-button-size)",
        "control-height": "var(--desktop-control-height)",
        "control-bg": "var(--control-surface)",
        "control-highlight": "var(--toolbar-control-highlight)",
        "control-border": "var(--control-edge)",
        "sidebar-stroke": "var(--control-edge)",
        "glass-sidebar-tint": "var(--navigation-backing)",
        "glass-toolbar-tint": "var(--navigation-backing)",
        "glass-text-secondary": "var(--navigation-muted)",
        "glass-control-bg": "var(--toolbar-control)",
        "sidebar-width": "var(--sidebar-width, var(--desktop-sidebar-width))",
    }
    lines.extend((":root .da-app, :root .da-app[data-theme],", ':root[data-theme] .da-app:not([data-theme="light"]):not([data-theme="dark"]) {'))
    lines.extend(f"  --da-{name}: {value};" for name, value in bridge.items())
    lines.extend(("  color-scheme: inherit;", "}", ""))
    return "\n".join(lines)


def icon_filename(name: str) -> str:
    return re.sub(r"(?<!^)([A-Z])", r"-\1", name).lower()


def validate_icons(manifest: dict) -> None:
    if manifest.get("schema") != 1 or manifest.get("viewBox") != "0 0 24 24" or manifest.get("strokeWidth") != 1.7:
        raise ValueError("unsupported icon geometry")
    if not manifest.get("icons"):
        raise ValueError("the shared icon set is empty")
    filenames = set()
    for name, paths in manifest["icons"].items():
        if not re.fullmatch(r"[a-z][a-zA-Z0-9]*", name) or not paths:
            raise ValueError(f"invalid icon name: {name}")
        filename = icon_filename(name)
        if filename in filenames:
            raise ValueError(f"duplicate icon filename: {filename}")
        filenames.add(filename)
        for path in paths:
            if not isinstance(path, str) or not re.fullmatch(r"[MmZzLlHhVvCcSsQqTtAaEe0-9+.,\s-]+", path):
                raise ValueError(f"invalid SVG path: {name}")


def render_icon(manifest: dict, name: str) -> str:
    paths = "".join(f'<path d="{escape(path, quote=True)}"/>' for path in manifest["icons"][name])
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{manifest["viewBox"]}" width="24" height="24" fill="none" stroke="currentColor" stroke-width="{manifest["strokeWidth"]}" stroke-linecap="round" stroke-linejoin="round">{paths}</svg>\n'


def generated_outputs(tokens: dict, manifest: dict) -> list[tuple[Path, str]]:
    return [
        (CSS, render_css(tokens)),
        (DESKTOP_CSS, render_desktop_css(tokens)),
        (PILL_METRICS, render_pill_metrics(tokens)),
        *((ICON_DIRECTORY / f"{icon_filename(name)}.svg", render_icon(manifest, name)) for name in manifest["icons"]),
    ]


class GeneratorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tokens = json.loads(TOKENS.read_text(encoding="utf-8"))

    def test_canonical_palette_and_aliases_are_valid(self) -> None:
        validate(self.tokens)
        self.assertEqual(self.tokens["aliases"]["paper"], "surface")
        self.assertEqual(self.tokens["aliases"]["action"], "button")

    def test_generation_only_targets_active_shared_outputs(self) -> None:
        manifest = json.loads(ICON_MANIFEST.read_text(encoding="utf-8"))
        paths = [path for path, _ in generated_outputs(self.tokens, manifest)]
        self.assertEqual(paths, [
            CSS,
            DESKTOP_CSS,
            PILL_METRICS,
            *(ICON_DIRECTORY / f"{icon_filename(name)}.svg" for name in manifest["icons"]),
        ])

    def test_unreadable_secondary_text_is_rejected(self) -> None:
        invalid = copy.deepcopy(self.tokens)
        invalid["colors"]["muted"]["light"] = "#AAAAAA"
        with self.assertRaisesRegex(ValueError, "contrast"):
            validate(invalid)

    def test_translucent_navigation_remains_readable_on_extreme_backdrops(self) -> None:
        for appearance in ("light", "dark"):
            for backdrop in ("#000000", "#FFFFFF"):
                backing = composite(self.tokens["colors"]["navigation-backing"][appearance], backdrop)
                self.assertGreaterEqual(contrast(self.tokens["colors"]["foreground"][appearance], backing), 4.5)
                self.assertGreaterEqual(contrast(self.tokens["colors"]["navigation-muted"][appearance], backing), 4.5)
        invalid = copy.deepcopy(self.tokens)
        invalid["colors"]["navigation-backing"]["light"] = "#F7F4EE00"
        with self.assertRaisesRegex(ValueError, "navigation text"):
            validate(invalid)

    def test_toolbar_labels_stay_readable_through_control_and_material_layers(self) -> None:
        for appearance in ("light", "dark"):
            for backdrop in ("#000000", "#FFFFFF"):
                backing = composite(self.tokens["colors"]["navigation-backing"][appearance], backdrop)
                for role in ("toolbar-control", "toolbar-control-hover"):
                    control = composite(self.tokens["colors"][role][appearance], backing)
                    self.assertGreaterEqual(contrast(self.tokens["colors"]["foreground"][appearance], control), 4.5)
        invalid = copy.deepcopy(self.tokens)
        invalid["colors"]["toolbar-control-hover"]["dark"] = "#FFFFFFFF"
        with self.assertRaisesRegex(ValueError, "toolbar control labels"):
            validate(invalid)

    def test_invalid_colors_and_aliases_are_rejected(self) -> None:
        invalid = copy.deepcopy(self.tokens)
        invalid["colors"]["surface"]["light"] = "white"
        with self.assertRaisesRegex(ValueError, "hex color"):
            validate(invalid)
        invalid = copy.deepcopy(self.tokens)
        invalid["aliases"]["paper"] = "missing"
        with self.assertRaisesRegex(ValueError, "alias"):
            validate(invalid)

    def test_pill_cannot_invert_with_the_window(self) -> None:
        invalid = copy.deepcopy(self.tokens)
        invalid["colors"]["pill-background"]["dark"] = "#202022"
        with self.assertRaisesRegex(ValueError, "remain stable"):
            validate(invalid)

    def test_generated_css_retains_semantic_values(self) -> None:
        css = render_css(self.tokens)
        self.assertIn("--surface: #fcfbf8;", css)
        self.assertIn("--surface: #1e1c1a;", css)
        self.assertEqual(css.split("/* Configurable pill variants")[0].count("--pill-background: #2a2723;"), 4)

    def test_pill_variants_generate_scoped_colours_and_validate_contrast(self) -> None:
        variants = copy.deepcopy(self.tokens)
        variants["pill-themes"] = {"ink": {"background": "#2A2723", "text": "#FCFBF8", "accent": "#F2B48E"}, "light": {"background": "#FCFBF8", "text": "#2A2723", "accent": "#87532B"}}
        css = render_css(variants)
        self.assertIn('[data-pill-colour="light"]', css)
        self.assertIn('[data-pill-colour="system"]', css)
        self.assertIn('[data-pill-appearance="light"]', css)
        variants["pill-themes"]["light"]["text"] = "#FCFBF8"
        with self.assertRaisesRegex(ValueError, "pill.*contrast"):
            validate(variants)

    def test_pill_size_variants_reject_invalid_geometry_and_emit_shared_metrics(self) -> None:
        variants = copy.deepcopy(self.tokens)
        variants["pill-sizes"] = {"compact": {**variants["pill"], "window-width": 360}, "regular": variants["pill"].copy(), "large": {**variants["pill"], "window-width": 500}}
        self.assertIn("export const PILL_SIZES", render_pill_metrics(variants))
        self.assertIn('[data-pill-size="compact"]', render_css(variants))
        variants["pill-sizes"]["compact"]["window-height"] = -1
        with self.assertRaisesRegex(ValueError, "pill.*metric"):
            validate(variants)

    def test_pill_frames_reserve_space_for_caption_and_structured_recovery(self) -> None:
        invalid = copy.deepcopy(self.tokens)
        invalid["pill-sizes"]["compact"]["window-height"] = 40
        with self.assertRaisesRegex(ValueError, "pill.*frame"):
            validate(invalid)

    def test_pill_theme_values_and_font_sizes_fail_with_readable_validation(self) -> None:
        invalid = copy.deepcopy(self.tokens)
        invalid["pill-themes"]["light"]["accent"] = 1
        with self.assertRaisesRegex(ValueError, "pill theme"):
            validate(invalid)
        invalid = copy.deepcopy(self.tokens)
        invalid["pill-sizes"]["large"]["label-size"] = 1
        with self.assertRaisesRegex(ValueError, "pill metric"):
            validate(invalid)

    def test_desktop_theme_does_not_change_mobile_or_pill_branding(self) -> None:
        base = copy.deepcopy(self.tokens)
        del base["desktop-theme"]
        self.assertEqual(render_css(self.tokens), render_css(base))
        self.assertEqual(render_pill_metrics(self.tokens), render_pill_metrics(base))
        self.assertEqual(self.tokens["typography"]["ui"], "Figtree")
        self.assertEqual(self.tokens["typography"]["display"], "Instrument Serif")
        self.assertEqual(self.tokens["colors"]["surface"], {"light": "#FCFBF8", "dark": "#1E1C1A"})
        self.assertEqual(self.tokens["radii"]["panel"], 12)
        desktop = render_desktop_css(self.tokens)
        self.assertIn('--ui: "Desktop Inter", -apple-system', desktop)
        self.assertIn('--serif: var(--ui);', desktop)
        self.assertIn("--surface: #ffffff;", desktop)
        self.assertIn("--surface: #101010;", desktop)
        self.assertIn("--error: var(--danger);", desktop)
        self.assertNotIn("--pill-", desktop)
        self.assertNotIn("--brand-", desktop)
        self.assertNotIn("--wave:", desktop)

    def test_desktop_theme_preserves_fixed_brand_and_font_constraints(self) -> None:
        invalid = copy.deepcopy(self.tokens)
        invalid["desktop-theme"]["colors"]["pill-background"] = {"light": "#000000", "dark": "#000000"}
        with self.assertRaisesRegex(ValueError, "fixed brand"):
            validate(invalid)
        invalid = copy.deepcopy(self.tokens)
        invalid["desktop-theme"]["typography"]["ui"] = "External font"
        with self.assertRaisesRegex(ValueError, "Desktop Inter"):
            validate(invalid)
        invalid = copy.deepcopy(self.tokens)
        invalid["typography"]["ui"] = "Desktop Inter"
        with self.assertRaisesRegex(ValueError, "bundled Pennel font"):
            validate(invalid)

    def test_desktop_theme_rejects_unreadable_chrome_and_semantic_states(self) -> None:
        for role, appearance, value in (("navigation-backing", "light", "#EFEFEF00"), ("muted", "light", "#AAAAAA"), ("danger", "dark", "#333333")):
            invalid = copy.deepcopy(self.tokens)
            invalid["desktop-theme"]["colors"][role][appearance] = value
            with self.subTest(role=role), self.assertRaisesRegex(ValueError, "contrast"):
                validate(invalid)

    def test_desktop_live_and_speaker_text_stay_readable_on_desktop_surfaces(self) -> None:
        validate(self.tokens)
        invalid = copy.deepcopy(self.tokens)
        invalid["desktop-theme"]["colors"]["live"]["light"] = "#E0703C"
        with self.assertRaisesRegex(ValueError, "live on surface"):
            validate(invalid)
        # Readable on the base paper and desktop white, but not on the desktop's quiet panels.
        invalid = copy.deepcopy(self.tokens)
        invalid["colors"]["voice-1"]["light"] = "#747474"
        with self.assertRaisesRegex(ValueError, "voice-1 on subtle"):
            validate(invalid)
        invalid = copy.deepcopy(self.tokens)
        del invalid["desktop-theme"]["colors"]["live"]
        with self.assertRaisesRegex(ValueError, "missing desktop semantic colors: live"):
            validate(invalid)

    def test_desktop_theme_requires_complete_semantic_roles(self) -> None:
        invalid = copy.deepcopy(self.tokens)
        del invalid["desktop-theme"]["colors"]["control-surface"]
        with self.assertRaisesRegex(ValueError, "missing desktop semantic colors"):
            validate(invalid)
        invalid = copy.deepcopy(self.tokens)
        del invalid["desktop-theme"]["radii"]["dialog"]
        with self.assertRaisesRegex(ValueError, "missing desktop radius"):
            validate(invalid)

    def test_desktop_theme_bridge_and_forced_appearance_share_the_semantic_palette(self) -> None:
        desktop = render_desktop_css(self.tokens)
        self.assertNotIn("light-dark(", desktop)
        self.assertIn(':root[data-appearance="light"]', desktop)
        self.assertIn(':root[data-appearance="dark"]', desktop)
        self.assertLess(desktop.index("@media (prefers-color-scheme: dark)"), desktop.index(':root[data-appearance="light"]'))
        self.assertIn("--da-accent: var(--button);", desktop)
        self.assertIn("--da-danger: var(--danger);", desktop)
        self.assertIn("--da-glass-sidebar-tint: var(--navigation-backing);", desktop)
        self.assertIn("--da-sidebar-width: var(--sidebar-width, var(--desktop-sidebar-width));", desktop)
        self.assertIn("color-scheme: inherit;", desktop)

    def test_interface_font_follows_the_original_figtree_and_can_use_the_system_font(self) -> None:
        css = render_css(self.tokens)
        self.assertIn('--ui: "Figtree", -apple-system', css)
        self.assertIn('--brand-ui: "Figtree"', css)
        system = copy.deepcopy(self.tokens)
        system["typography"]["interface"] = "system"
        self.assertIn('--ui: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui', render_css(system))
        invalid = copy.deepcopy(self.tokens)
        invalid["typography"]["interface"] = "comic"
        with self.assertRaisesRegex(ValueError, "interface"):
            validate(invalid)

    def test_pill_geometry_is_shared_by_css_and_frontend_state(self) -> None:
        css, javascript = render_css(self.tokens), render_pill_metrics(self.tokens)
        for name, value in self.tokens["pill"].items():
            unit = "" if name in PILL_COUNTS else "px"
            self.assertIn(f"--pill-{name}: {scalar(value)}{unit};", css)
            self.assertIn(f"{camel_name(name)}: {scalar(value)},", javascript)

    def test_desktop_chrome_keeps_reading_fonts_and_shared_sidebar_bounds(self) -> None:
        css = render_css(self.tokens)
        self.assertIn('--ui: "Figtree"', css)
        self.assertEqual(self.tokens["typography"]["macos-chrome"], "brand")
        self.assertIn('--chrome-ui: var(--ui);', css)
        self.assertNotIn(':root[data-platform="macos"]', css)
        for name, value in self.tokens["desktop"].items():
            self.assertIn(f"--desktop-{name}: {scalar(value)}px", css)
        invalid = copy.deepcopy(self.tokens)
        invalid["desktop"]["sidebar-width"] = invalid["desktop"]["sidebar-max-width"] + 1
        with self.assertRaisesRegex(ValueError, "within its bounds"):
            validate(invalid)
        invalid = copy.deepcopy(self.tokens)
        invalid["pill"]["preview-lines"] = 1.5
        with self.assertRaisesRegex(ValueError, "pill count"):
            validate(invalid)

    def test_system_and_forced_appearances_support_minimum_webkit(self) -> None:
        css = render_css(self.tokens)
        self.assertNotIn("light-dark(", css)
        media = css.index("@media (prefers-color-scheme: dark)")
        light = css.index(':root[data-appearance="light"]')
        dark = css.index(':root[data-appearance="dark"]')
        self.assertLess(media, light)
        self.assertLess(light, dark)
        system_block = css[media:light]
        light_block = css[light:dark]
        dark_block = css[dark:]
        for name, pair in self.tokens["colors"].items():
            self.assertIn(f"--{name}: {pair['light'].lower()};", css[:media])
            self.assertIn(f"--{name}: {pair['dark'].lower()};", system_block)
            self.assertIn(f"--{name}: {pair['light'].lower()};", light_block)
            self.assertIn(f"--{name}: {pair['dark'].lower()};", dark_block)

    def test_icon_shapes_are_shared_and_resource_safe(self) -> None:
        manifest = json.loads(ICON_MANIFEST.read_text(encoding="utf-8"))
        validate_icons(manifest)
        svg = render_icon(manifest, "arrowRight").strip()
        self.assertEqual(icon_filename("arrowRight"), "arrow-right")
        self.assertIn('stroke-width="1.7"', svg)
        invalid = copy.deepcopy(manifest)
        invalid["icons"]["mic"] = ['M0 0" onclick="bad']
        with self.assertRaisesRegex(ValueError, "SVG path"):
            validate_icons(invalid)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="validate tokens and verify generated files are current")
    mode.add_argument("--self-test", action="store_true", help="run generator and palette regression tests without writing files")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(GeneratorTests))
        return 0 if result.wasSuccessful() else 1
    try:
        tokens = json.loads(TOKENS.read_text(encoding="utf-8"))
        validate(tokens)
        manifest = json.loads(ICON_MANIFEST.read_text(encoding="utf-8"))
        validate_icons(manifest)
        outputs = generated_outputs(tokens, manifest)
        stale = []
        for path, content in outputs:
            if path.exists() and path.read_text(encoding="utf-8") == content:
                continue
            if args.check:
                stale.append(str(path.relative_to(ROOT)))
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")
        if stale:
            print("Theme output is stale; run python3 scripts/generate-theme.py:", file=sys.stderr)
            print("\n".join(stale), file=sys.stderr)
            return 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"Theme generation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
