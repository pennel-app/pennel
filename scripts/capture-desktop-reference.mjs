#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, mkdir, mkdtemp, open, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { visualModels, visualCallApps } from "./fixtures/model-catalog.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appRoot = join(root, "apps/desktop");
const fixtureRoot = join(root, "scripts/fixtures");
const cases = JSON.parse(await readFile(join(fixtureRoot, "visual-cases.json"), "utf8"));
const fixtureScript = await readFile(join(fixtureRoot, "desktop-platform.js"), "utf8");
const localModels = visualModels(JSON.parse(await readFile(join(root, "core/src/catalog.json"), "utf8")), await readFile(join(root, "core/src/catalog.rs"), "utf8"));
const recordingAppIds = visualCallApps(await readFile(join(root, "core/src/rules.rs"), "utf8"));
const pillMetrics = JSON.parse(await readFile(join(root, "assets/brand/tokens.json"), "utf8")).pill;
const argumentsList = process.argv.slice(2);
const options = { output: join(root, "build/visual-parity/desktop"), appearance: "both", locale: "both", cases: [], port: 0, serve: false, browser: process.env.PENNEL_CHROME, baselineRef: null };
for (let index = 0; index < argumentsList.length; index++) {
  const name = argumentsList[index];
  if (name === "--serve") options.serve = true;
  else if (name === "--output") options.output = resolve(argumentsList[++index]);
  else if (name === "--case") options.cases.push(argumentsList[++index]);
  else if (name === "--appearance") options.appearance = argumentsList[++index];
  else if (name === "--locale") options.locale = argumentsList[++index];
  else if (name === "--port") options.port = Number(argumentsList[++index]);
  else if (name === "--browser") options.browser = argumentsList[++index];
  else if (name === "--baseline-ref") options.baselineRef = argumentsList[++index];
  else if (name === "--list") { console.log(cases.map(item => item.name).join("\n")); process.exit(0); }
  else throw new Error(`Unknown option: ${name}`);
}
if (!["both", "light", "dark"].includes(options.appearance)) throw new Error("--appearance must be light, dark, or both");
if (!["both", "en", "nl"].includes(options.locale)) throw new Error("--locale must be en, nl, or both");
const locales = options.locale === "both" ? ["en", "nl"] : [options.locale];
const selected = options.cases.length ? cases.filter(item => options.cases.includes(item.name)) : cases;
if (!selected.length || options.cases.some(name => !cases.some(item => item.name === name))) throw new Error("Unknown or empty visual case selection");
const appearances = options.appearance === "both" ? ["light", "dark"] : [options.appearance];
let baselineCommit = null;
if (options.baselineRef !== null) {
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._/-]*$/.test(options.baselineRef)) throw new Error("Invalid --baseline-ref");
  baselineCommit = execFileSync("git", ["rev-parse", "--verify", `${options.baselineRef}^{commit}`], { cwd: root, encoding: "utf8" }).trim();
}
const meetingId = "2026-10-24 0900 Projectoverleg";
// Default sizes of the shared app's windows (apps/desktop/src-tauri/src/workspace.rs).
const windowSizes = { workspace: [980, 640], settings: [980, 640], history: [980, 640], dictation: [980, 640], onboarding: [640, 660] };
const urlFor = (base, item, appearance, locale) => {
  const url = new URL(item.window === "pill" ? "/pill.html" : "/index.html", base);
  for (const [key, value] of Object.entries({ qa: item.name, theme: appearance, locale, window: item.window, section: item.section, tab: item.tab, meeting_id: item.meeting_tab ? meetingId : undefined, meeting_tab: item.meeting_tab, profile: item.profile, preview: item.pill_demo ? "1" : undefined })) if (value) url.searchParams.set(key, value);
  return url.href;
};
const { createServer } = await import(pathToFileURL(join(appRoot, "node_modules/vite/dist/node/index.js")).href);
const viteCache = await mkdtemp(join(tmpdir(), "pennel-visual-vite-"));
const server = await createServer({
  configFile: join(appRoot, "vite.config.ts"), root: appRoot, cacheDir: viteCache,
  server: { host: "127.0.0.1", port: options.port, strictPort: Boolean(options.port), hmr: false, fs: { allow: [root] } },
  plugins: [{
    name: "pennel-owned-baseline-source",
    enforce: "pre",
    load(id) {
      if (!baselineCommit || id.includes("?")) return;
      const path = relative(root, id).replaceAll("\\", "/");
      if (!/^(apps\/desktop\/src\/.*\.(ts|css)|assets\/locales\/.*\.json)$/.test(path)) return;
      try { return execFileSync("git", ["show", `${baselineCommit}:${path}`], { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }); }
      catch { return; } // Generated, ignored frontend assets retain their current canonical source.
    },
  }, {
    name: "pennel-owned-visual-fixtures",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        if (request.url?.split("?")[0] !== "/favicon.ico") return next();
        response.statusCode = 204;
        response.end();
      });
    },
    transformIndexHtml: {
      order: "pre",
      handler(_html, context) {
        const url = new URL(context.originalUrl || context.path, "http://127.0.0.1");
        const item = cases.find(item => item.name === url.searchParams.get("qa"));
        if (!item) return;
        const appearance = url.searchParams.get("theme") === "dark" ? "dark" : "light";
        const data = JSON.stringify({ ...item, appearance, locale: url.searchParams.get("locale") || "nl", ui_language: url.searchParams.get("ui_language") || undefined, profile: url.searchParams.get("profile") || undefined, conversation_fixture_url: `/@fs/${join(root, "scripts/fixtures/meeting-conversations.mjs").replaceAll("\\", "/")}`, model_state_delay_ms: Math.min(2000, Math.max(0, Number(url.searchParams.get("model_state_delay_ms")) || 0)), local_models: localModels, recording_app_ids: recordingAppIds }).replaceAll("<", "\\u003c");
        const reset = item.owner_review && url.searchParams.get("review_reset") === "1"
          ? `\nif (globalThis.__PENNEL_FIXTURE__?.review) { globalThis.__PENNEL_FIXTURE__.review.resetProfile(); const clean = new URL(location.href); clean.searchParams.delete("review_reset"); location.replace(clean.href); }`
          : "";
        return [{ tag: "script", attrs: { "data-pennel-fixture": item.name }, children: `globalThis.__PENNEL_FIXTURE_OPTIONS__=${data};\n${fixtureScript}${reset}`, injectTo: "head-prepend" }];
      },
    },
  }],
});
await server.listen();
const address = server.httpServer.address();
const base = `http://127.0.0.1:${address.port}`;
let browser;
let profile;
let connection;
let closing = false;
async function cleanup() {
  if (closing) return;
  closing = true;
  connection?.close();
  if (browser && browser.exitCode === null) {
    browser.kill("SIGTERM");
    await Promise.race([new Promise(resolve => browser.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 2000))]);
    if (browser.exitCode === null) browser.kill("SIGKILL");
  }
  await server.close();
  if (profile) await rm(profile, { recursive: true, force: true });
  await rm(viteCache, { recursive: true, force: true });
}
process.on("SIGINT", () => { void cleanup().finally(() => process.exit(0)); });
process.on("SIGTERM", () => { void cleanup().finally(() => process.exit(0)); });

if (options.serve) {
  console.log(JSON.stringify({ url: base, example: urlFor(base, selected[0], appearances[0], locales[0]), cases: selected.length, appearances, locales }, null, 2));
  await new Promise(() => {});
} else {
  let captureManifest;
  let currentCase = "browser-start";
  try {
    await mkdir(options.output, { recursive: true });
    captureManifest = { captured_at: new Date().toISOString(), complete: false, capture_method: "Owned Chromium headless fixture render via CDP; no user screen/profile", baseline_commit: baselineCommit,
      fixture_sha256: createHash("sha256").update(JSON.stringify({ cases, fixtureScript, localModels, recordingAppIds })).digest("hex"), fixture_clock: "2026-10-25T09:00:00Z", images: [] };
    await writeFile(join(options.output, "manifest.json"), JSON.stringify(captureManifest, null, 2) + "\n");
    const candidates = [options.browser, "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium", "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"].filter(Boolean);
    let resolvedBrowser;
    for (const candidate of candidates) { try { await access(candidate); resolvedBrowser = candidate; break; } catch {} }
    if (!resolvedBrowser) throw new Error("No local Chromium found. Set PENNEL_CHROME or use --browser; --serve works with the existing browser tools.");
    options.browser = resolvedBrowser;
    captureManifest.browser = basename(options.browser);
    profile = await mkdtemp(join(tmpdir(), "pennel-visual-chrome-"));
    const log = await open(join(options.output, "browser.log"), "w");
    browser = spawn(options.browser, ["--headless=new", "--disable-gpu", "--hide-scrollbars", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--no-first-run", "--no-default-browser-check", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank"], { stdio: ["ignore", "ignore", log.fd] });
    await log.close();
    let port;
    for (let attempt = 0; attempt < 100; attempt++) {
      try { port = Number((await readFile(join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]); if (port) break; } catch {}
      if (browser.exitCode !== null) throw new Error(`Chromium exited with ${browser.exitCode}; inspect browser.log`);
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    if (!port) throw new Error("Chromium did not expose its owned debugging endpoint");
    const version = await fetch(`http://127.0.0.1:${port}/json/version`).then(response => response.json());
    connection = await connectCDP(version.webSocketDebuggerUrl);
    const manifest = captureManifest;
    for (const locale of locales) {
      for (const appearance of appearances) {
      await mkdir(join(options.output, locale, appearance), { recursive: true });
      for (const item of selected) {
        currentCase = `${locale}/${appearance}/${item.name}`;
        // A scenario keeps its own reload state without inheriting another scenario's recovery or layout settings.
        const { browserContextId } = await connection.send("Target.createBrowserContext", { disposeOnDetach: true });
        try {
        const { targetId } = await connection.send("Target.createTarget", { url: "about:blank", browserContextId });
        const { sessionId } = await connection.send("Target.attachToTarget", { targetId, flatten: true });
        const send = (method, params = {}) => connection.send(method, params, sessionId);
        await send("Page.enable");
        await send("Runtime.enable");
        await send("Log.enable");
        await send("Emulation.setTimezoneOverride", { timezoneId: "Europe/Brussels" });
        await send("Emulation.setLocaleOverride", { locale: locale === "nl" ? "nl-BE" : "en-GB" });
        await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: appearance }, ...(item.reduced_motion ? [{ name: "prefers-reduced-motion", value: "reduce" }] : [])] });
        connection.events.length = 0;
        const [width, height] = item.size ?? (item.window === "pill" ? [pillMetrics["window-width"], pillMetrics["window-height"]] : windowSizes[item.window]);
        await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: item.scale ?? 1, mobile: false });
        await send("Emulation.setDefaultBackgroundColorOverride", item.window === "pill" ? { color: { r: 0, g: 0, b: 0, a: 0 } } : {});
        const url = urlFor(base, item, appearance, locale);
        await send("Page.navigate", { url });
        let rendered = false;
        for (let attempt = 0; attempt < 100; attempt++) {
          try {
            const status = await send("Runtime.evaluate", { expression: item.ready_when || (item.window === "pill"
              ? "Boolean(window.__PENNEL_FIXTURE__ && document.getElementById('pill') && !document.getElementById('pill').hidden && document.getElementById('label')?.textContent.trim())"
              : "Boolean(window.__PENNEL_FIXTURE__ && document.getElementById('app')?.textContent.trim())"), returnByValue: true });
            if (status.result.value) { rendered = true; break; }
          } catch (error) { if (!String(error).includes("context")) throw error; }
          await new Promise(resolve => setTimeout(resolve, 100));
        }
        if (!rendered) throw new Error(`${item.name}: production entry did not render using the fixture seam`);
        await send("Runtime.evaluate", { expression: "(async()=>{await document.fonts.ready;await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));})()", awaitPromise: true, returnByValue: true });
        await new Promise(resolve => setTimeout(resolve, 250));
        for (const step of item.steps || []) {
          if (step.reload) {
            const previous = (await send("Page.getFrameTree")).frameTree.frame.loaderId;
            await send("Page.reload", { ignoreCache: true });
            let ready = false;
            for (let attempt = 0; attempt < 100; attempt++) {
              try {
                if ((await send("Page.getFrameTree")).frameTree.frame.loaderId !== previous) {
                  const state = await send("Runtime.evaluate", { expression: "Boolean(window.__PENNEL_FIXTURE__ && document.getElementById('app')?.textContent.trim())", returnByValue: true });
                  if (state.result.value) { ready = true; break; }
                }
              } catch (error) { if (!String(error).includes("context")) throw error; }
              await new Promise(resolve => setTimeout(resolve, 100));
            }
            if (!ready) throw Error(`${item.name}: the owned fixture did not reopen after reload`);
            await send("Runtime.evaluate", { expression: "(async()=>{await document.fonts.ready;await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));})()", awaitPromise: true });
          } else if (step.viewport) {
            const [width, height] = step.viewport;
            if (![width, height].every(value => Number.isInteger(value) && value >= 240 && value <= 10000)) throw Error("Invalid owned fixture viewport");
            await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: item.scale ?? 1, mobile: false });
          } else if (step.pointer) {
            // Real pointer input, so :hover and :active apply: { pointer: "<selector>", action: "move" | "press" | "release" }.
            const target = await send("Runtime.evaluate", { expression: `(()=>{const r=document.querySelector(${JSON.stringify(step.pointer)})?.getBoundingClientRect();return r&&{x:r.x+r.width/2,y:r.y+r.height/2}})()`, returnByValue: true });
            if (!target.result.value) throw new Error(`${item.name}: no element for pointer step ${step.pointer}`);
            const { x, y } = target.result.value;
            const action = step.action || "move";
            if (action === "press") await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
            await send("Input.dispatchMouseEvent", action === "move" ? { type: "mouseMoved", x, y } : { type: action === "press" ? "mousePressed" : "mouseReleased", x, y, button: "left", buttons: action === "press" ? 1 : 0, clickCount: 1 });
          } else if (step.key) {
            const key = { key: step.key, code: step.code || step.key, windowsVirtualKeyCode: step.keyCode };
            const text = step.text ?? (step.key === "Enter" ? "\r" : step.key.length === 1 ? step.key : undefined);
            await send("Input.dispatchKeyEvent", { type: "keyDown", ...key, ...text ? { text, unmodifiedText: text } : {} });
            await send("Input.dispatchKeyEvent", { type: "keyUp", ...key });
          } else {
            const result = await send("Runtime.evaluate", { expression: step.eval, awaitPromise: true });
            if (result.exceptionDetails) {
              const failed = await send("Page.captureScreenshot", { format: "png", fromSurface: true, captureBeyondViewport: false });
              await writeFile(join(options.output, locale, appearance, `${item.name}-failure.png`), Buffer.from(failed.data, "base64"));
              throw new Error(`${item.name}: ${result.exceptionDetails.exception?.description || result.exceptionDetails.text}`);
            }
          }
          await new Promise(resolve => setTimeout(resolve, step.wait || 300));
        }
        const metadata = await send("Runtime.evaluate", { expression: "(()=>{const pill=document.getElementById('pill'),preview=document.getElementById('preview'),text=document.getElementById('preview-text'),bounds=pill?.getBoundingClientRect();const pillError=pill?(bounds.left<0||bounds.top<0||bounds.right>innerWidth||bounds.bottom>innerHeight?'Pill exceeds the viewport':text?.querySelector('*')?'Preview interpreted markup':window.__PENNEL_FIXTURE__.options.preview_text&&preview.hidden?'Expected preview is hidden':''):'';return {document_language:document.documentElement.lang,width:innerWidth,height:innerHeight,horizontal_overflow:document.documentElement.scrollWidth>innerWidth,unknown_commands:window.__PENNEL_FIXTURE__.unknown,calls:window.__PENNEL_FIXTURE__.calls,runtime_banner:document.querySelector('.notice-banner:not([hidden]):not([data-recovery-banner])')?.textContent||'',recovery_banner:document.querySelector('[data-recovery-banner]:not([hidden])')?.textContent||'',surface_error:pillError,pill_preview:text?{text:text.textContent,visible:!preview.hidden,height:preview.getBoundingClientRect().height}:null,body:document.body.innerText.slice(0,300)}})()", returnByValue: true });
        for (const [selector, expected] of item.expect_font_size || []) {
          const measured = await send("Runtime.evaluate", { expression: `(()=>{const element=document.querySelector(${JSON.stringify(selector)});return element?parseFloat(getComputedStyle(element).fontSize):null})()`, returnByValue: true });
          if (measured.result.value !== expected) throw new Error(`${item.name}: ${selector} computed font-size ${measured.result.value} px, expected ${expected} px`);
        }
        const acceptance = await send("Runtime.evaluate", { expression: "window.__PENNEL_LIVE_ACCEPTANCE__ || null", returnByValue: true });
        metadata.result.value.live_acceptance = acceptance.result.value;
        const peopleAcceptance = await send("Runtime.evaluate", { expression: "window.__PENNEL_PEOPLE_ACCEPTANCE__ || null", returnByValue: true });
        metadata.result.value.people_acceptance = peopleAcceptance.result.value;
        const conversationAcceptance = await send("Runtime.evaluate", { expression: "window.__PENNEL_CONVERSATION_ACCEPTANCE__ || null", returnByValue: true });
        metadata.result.value.conversation_acceptance = conversationAcceptance.result.value;
        const controlAcceptance = await send("Runtime.evaluate", { expression: "window.__PENNEL_CONTROL_ACCEPTANCE__ || null", returnByValue: true });
        metadata.result.value.control_acceptance = controlAcceptance.result.value;
        const pillAcceptance = await send("Runtime.evaluate", { expression: "window.__PENNEL_PILL_ACCEPTANCE__ || null", returnByValue: true });
        metadata.result.value.pill_acceptance = pillAcceptance.result.value;
        const image = await send("Page.captureScreenshot", { format: "png", fromSurface: true, captureBeyondViewport: false });
        const file = `${locale}/${appearance}/${item.name}.png`;
        await writeFile(join(options.output, file), Buffer.from(image.data, "base64"));
        manifest.images.push({ case: item.name, fixture_case_sha256: createHash("sha256").update(JSON.stringify(item)).digest("hex"), locale, appearance, file, ...metadata.result.value, runtime_errors: [...connection.events] });
        await writeFile(join(options.output, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
        if (metadata.result.value.unknown_commands.length) console.error(`${item.name}: unknown fixture commands ${metadata.result.value.unknown_commands.join(", ")}`);
        } finally {
          await connection.send("Target.disposeBrowserContext", { browserContextId });
        }
      }
    }
    }
    manifest.complete = true;
    await writeFile(join(options.output, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
    const failures = manifest.images.filter(item => item.horizontal_overflow || item.unknown_commands.length || item.runtime_errors.length || item.runtime_banner || item.surface_error || item.document_language !== item.locale);
    console.log(JSON.stringify({ output: options.output, images: manifest.images.length, failures: failures.map(item => ({ case: item.case, locale: item.locale, appearance: item.appearance, document_language: item.document_language, horizontal_overflow: item.horizontal_overflow, unknown_commands: item.unknown_commands, runtime_errors: item.runtime_errors, runtime_banner: item.runtime_banner, surface_error: item.surface_error })) }, null, 2));
    process.exitCode = failures.length ? 1 : 0;
  } catch (error) {
    if (captureManifest) {
      captureManifest.failure = { case: currentCase, message: String(error) };
      await writeFile(join(options.output, "manifest.json"), JSON.stringify(captureManifest, null, 2) + "\n");
    }
    throw new Error(`Visual capture failed at ${currentCase}: ${error.message}`, { cause: error });
  } finally { await cleanup(); }
}

async function connectCDP(url) {
  const socket = new WebSocket(url);
  let nextId = 0;
  const pending = new Map();
  const events = [];
  socket.addEventListener("message", event => {
    const message = JSON.parse(event.data);
    if (message.method === "Runtime.exceptionThrown") events.push({ kind: "exception", message: String(message.params.exceptionDetails.exception?.description || message.params.exceptionDetails.text).slice(0,600) });
    if (message.method === "Log.entryAdded" && message.params.entry.level === "error") events.push({ kind: "log", message: message.params.entry.text.slice(0,600), url: message.params.entry.url || null });
    const waiting = pending.get(message.id);
    if (!waiting) return;
    clearTimeout(waiting.timer); pending.delete(message.id);
    if (message.error) waiting.reject(new Error(`${waiting.method}: ${message.error.message}`));
    else waiting.resolve(message.result);
  });
  await new Promise((resolve, reject) => { socket.addEventListener("open", resolve, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  return {
    events,
    send(method, params = {}, sessionId) {
      const id = ++nextId;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 15000);
        pending.set(id, { resolve, reject, timer, method });
        socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
      });
    },
    close() { socket.close(); },
  };
}
