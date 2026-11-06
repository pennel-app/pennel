#!/usr/bin/env node
import { spawn, execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const issue = (number, title) => ({ title, url: `https://github.com/pennel-app/pennel/issues/${number}` });
const scenario = (name, label, section, extra = {}) => ({ name, label, route: { window: "workspace", section, ...extra } });

export const reviewFlows = {
  navigation: {
    issue: issue(144, "Does the two-space navigation make both jobs obvious?"),
    scenarios: [
      scenario("owner-review-navigation", "Everyday work", "dictation"),
      scenario("owner-review-recording", "An active meeting", "recording"),
      scenario("owner-review-history-recovery", "A retained dictation", "history", { window: "history" }),
      { name: "owner-review-practice", label: "Owned voice practice", route: { window: "onboarding", tab: "practice" } },
    ],
    tasks: [
      { en: "Prepare an editable voice-written draft, find an earlier result, then continue your draft.", nl: "Maak een bewerkbaar concept met je stem, zoek een eerder resultaat en ga daarna verder met je concept." },
      { en: "Find the earlier conversation about the client, then return to the work you were doing.", nl: "Zoek het eerdere gesprek over de klant en ga daarna terug naar je werk." },
      { en: "A meeting is still being captured. Look elsewhere in Pennel, then return to that recording.", nl: "Er loopt nog een opname. Bekijk iets anders in Pennel en keer daarna terug naar die opname." },
      { en: "The previous dictation could not be saved. Keep the useful text and continue.", nl: "Het vorige dictaat kon niet worden bewaard. Behoud de bruikbare tekst en ga verder." },
    ],
  },
  meeting: {
    issue: issue(145, "Does one notes-first workspace cover capture through review?"),
    scenarios: [
      scenario("owner-review-meeting", "Capture and the next meeting", "meetings"),
      scenario("owner-review-meeting-document", "Review a saved meeting", "meetings", { meeting_tab: "notes", meeting_id: "2026-10-24 0900 Projectoverleg" }),
      scenario("owner-review-meeting-recovery", "A save failure", "recording"),
      scenario("owner-review-meeting-setup", "Missing summary setup", "meetings", { meeting_tab: "notes", meeting_id: "2026-10-24 0900 Projectoverleg" }),
    ],
    tasks: [
      { en: "Capture the supplied conversation, add one note, finish it and capture the next conversation.", nl: "Neem het voorbeeldgesprek op, voeg een notitie toe, rond het af en neem het volgende gesprek op." },
      { en: "Turn the saved conversation into a concise account, adjust the wording and check what supports a point.", nl: "Maak een beknopt verslag van het bewaarde gesprek, pas de formulering aan en controleer waarop een punt is gebaseerd." },
      { en: "Prepare an editable copy for somebody else and check exactly what it contains.", nl: "Maak een bewerkbare kopie voor iemand anders en controleer precies wat erin staat." },
      { en: "The previous operation did not finish. Keep your writing, recover it and continue in the same document.", nl: "De vorige bewerking is niet afgerond. Behoud je tekst, herstel het probleem en werk verder in hetzelfde document." },
    ],
  },
  setup: {
    issue: issue(146, "Does task-based setup explain readiness and processing destinations?"),
    scenarios: [
      { name: "owner-review-setup-new", label: "First use", route: { window: "onboarding" } },
      scenario("owner-review-setup-upgraded", "An existing profile", "settings", { tab: "dictation" }),
      { name: "owner-review-setup-download", label: "A failed download", route: { window: "onboarding" } },
    ],
    tasks: [
      { en: "Set up Pennel for your first voice-written draft using the recommended option.", nl: "Stel Pennel in voor je eerste gesproken concept met de aanbevolen optie." },
      { en: "Explain where sound and written text will be processed, then inspect another available setup.", nl: "Leg uit waar geluid en tekst worden verwerkt en bekijk daarna een andere beschikbare instelling." },
      { en: "Try changing a setup, leave without applying it, then check which tasks still use the saved choices.", nl: "Probeer een instelling te wijzigen, ga terug zonder die toe te passen en controleer welke taken de bewaarde keuzes gebruiken." },
      { en: "Choose when meeting summaries should be made and recover the failed download.", nl: "Kies wanneer vergaderverslagen moeten worden gemaakt en herstel de mislukte download." },
    ],
  },
};

export function parseReviewArguments(args) {
  const options = { flow: "navigation", locale: "en", appearance: "light", port: 0, help: false };
  for (let index = 0; index < args.length; index++) {
    const flag = args[index];
    if (flag === "--help" || flag === "-h") { options.help = true; continue; }
    const key = { "--flow": "flow", "--locale": "locale", "--appearance": "appearance", "--port": "port" }[flag];
    if (!key) throw new Error(`Unknown review option: ${flag}`);
    const value = args[++index];
    if (!value || value.startsWith("--")) throw new Error(`Missing value for ${flag}`);
    options[key] = key === "port" ? Number(value) : value;
  }
  if (!Object.hasOwn(reviewFlows, options.flow)) throw new Error("--flow must be navigation, meeting, or setup");
  if (!["en", "nl"].includes(options.locale)) throw new Error("--locale must be en or nl");
  if (!["light", "dark"].includes(options.appearance)) throw new Error("--appearance must be light or dark");
  if (!Number.isInteger(options.port) || options.port < 0 || options.port > 65535) throw new Error("--port must be an integer from 0 through 65535");
  return options;
}

export function reviewLinks(base, options, session) {
  const origin = new URL(base);
  if (origin.protocol !== "http:" || !["127.0.0.1", "localhost"].includes(origin.hostname) || origin.username || origin.password) throw new Error("Review fixtures require a loopback HTTP server");
  if (!/^review-[a-z0-9-]+$/i.test(session)) throw new Error("Invalid review session identifier");
  return reviewFlows[options.flow].scenarios.map(item => {
    const url = new URL("/index.html", origin);
    for (const [key, value] of Object.entries({ ...item.route, qa: item.name, locale: options.locale, theme: options.appearance, profile: `${session}-${item.name}` })) url.searchParams.set(key, value);
    const reset = new URL(url);
    reset.searchParams.set("review_reset", "1");
    return { name: item.name, label: item.label, url: url.href, reset: reset.href };
  });
}

function candidate() {
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
  return { commit: git("rev-parse", "HEAD"), branch: git("branch", "--show-current"), dirty: Boolean(git("status", "--porcelain", "--untracked-files=normal")) };
}

async function run(options) {
  const flow = reviewFlows[options.flow], session = `review-${randomUUID()}`;
  const args = [join(root, "scripts/capture-desktop-reference.mjs"), "--serve", "--locale", options.locale, "--appearance", options.appearance, "--port", String(options.port)];
  for (const item of flow.scenarios) args.push("--case", item.name);
  const child = spawn(process.execPath, args, { cwd: root, stdio: ["ignore", "pipe", "inherit"] });
  const stop = () => { if (child.exitCode === null) child.kill("SIGTERM"); };
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  try {
    const server = await new Promise((resolve, reject) => {
      let output = "";
      const timer = setTimeout(() => { stop(); reject(new Error("The fixture server did not become ready within 30 seconds")); }, 30_000);
      child.once("error", error => { clearTimeout(timer); reject(error); });
      child.once("exit", code => { clearTimeout(timer); reject(new Error(`The fixture server exited before readiness (${code})`)); });
      child.stdout.on("data", chunk => {
        output += chunk;
        try { const ready = JSON.parse(output); if (ready.url) { clearTimeout(timer); resolve(ready); } } catch { /* The readiness JSON can span multiple chunks. */ }
      });
    });
    const links = reviewLinks(server.url, options, session);
    const manifest = { session, created_at: new Date().toISOString(), candidate: candidate(), flow: options.flow, locale: options.locale, appearance: options.appearance,
      evidence_kind: "synthetic-browser-owner-review-preparation", decision: flow.issue, scenarios: links,
      observation_template: "docs/usability/session-sheet.csv", human_outcomes: "not recorded" };
    const output = join(root, "build/desktop-ux-review", session);
    await mkdir(output, { recursive: true });
    await writeFile(join(output, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
    console.log(`${flow.issue.title}\n${flow.issue.url}\n\nCandidate: ${manifest.candidate.commit}${manifest.candidate.dirty ? " + uncommitted changes" : ""}\nSession: ${session}\nInterface: ${options.locale} / ${options.appearance}\nEvidence: simulated browser workflows; human outcomes have not been recorded.\n`);
    for (const item of links) console.log(`${item.label}\n  Open: ${item.url}\n  Reset only this scenario: ${item.reset}\n`);
    console.log("Task goals (present one at a time):");
    flow.tasks.forEach((task, index) => console.log(`${index + 1}. ${task[options.locale]}`));
    console.log(`\nFacilitator and native-review instructions: ${join(root, "docs/desktop-ux-owner-review.md")}\nBlank worksheet: ${join(root, "docs/usability/session-sheet.csv")}\nPreparation manifest: ${join(output, "manifest.json")}\nKeep completed worksheets in private local storage. No observations are collected or uploaded.\n\nLeave this command running; Ctrl+C stops its server.`);
    await new Promise((resolve, reject) => {
      if (child.exitCode !== null) return child.exitCode === 0 ? resolve() : reject(new Error(`Fixture server exited (${child.exitCode})`));
      child.once("exit", (code, signal) => code === 0 || signal === "SIGTERM" ? resolve() : reject(new Error(`Fixture server exited (${code ?? signal})`)));
    });
  } finally {
    stop();
    process.removeListener("SIGINT", stop);
    process.removeListener("SIGTERM", stop);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = parseReviewArguments(process.argv.slice(2));
    if (options.help) console.log("node scripts/review-desktop-ux.mjs [--flow navigation|meeting|setup] [--locale en|nl] [--appearance light|dark] [--port 0..65535]\nDefaults: navigation, en, light, fresh loopback port. Requires apps/desktop dependencies (npm ci). See docs/desktop-ux-owner-review.md.");
    else await run(options);
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
