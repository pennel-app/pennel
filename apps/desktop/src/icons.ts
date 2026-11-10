import manifestSource from "../../../assets/ui/icons/manifest.json?raw";

export type IconName = "sidebar" | "search" | "mic" | "notebook" | "history" | "settings" | "sun" | "moon" | "arrowRight" | "arrowLeft" | "chevronRight" | "copy" | "laptop" | "download" | "check" | "close" | "globe" | "square" | "rotate" | "warning" | "keyboard" | "audioLines" | "trash" | "chat" | "more";
const manifest = JSON.parse(manifestSource) as { viewBox: string; strokeWidth: number; icons: Record<IconName, string[]> };

export function icon(name: IconName): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  for (const [key, value] of Object.entries({ viewBox: manifest.viewBox, fill: "none", stroke: "currentColor", "stroke-width": String(manifest.strokeWidth), "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true", class: "ui-icon" })) svg.setAttribute(key, value);
  for (const shape of manifest.icons[name]) {
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", shape);
    svg.append(path);
  }
  return svg;
}
