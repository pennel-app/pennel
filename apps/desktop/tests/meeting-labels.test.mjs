import assert from "node:assert/strict";
import test from "node:test";
import { setInterfaceLanguage } from "../src/i18n.ts";
import { dayGroup, meetingDate, sourceSpeakerLabel, speakerLabel, speakerUiLabel, templateLabel, templateUiLabel } from "../src/meeting-labels.ts";

const others = { speaker: "others", voice: 2 };

test("automatic names retain their origin and manual names take precedence", () => {
  setInterfaceLanguage("en");
  const segment = { ...others, automatic_speaker_name: "Sam", attribution: { kind: "single", voices: [2], self_voice: false, provisional: false } };
  assert.equal(String(speakerUiLabel(segment)), "Sam (Teams estimate)");
  assert.equal(String(speakerUiLabel({ ...segment, confirmed_speaker_name: "Manual name" })), "Manual name");
  assert.notEqual(String(speakerUiLabel({ ...segment, attribution: { ...segment.attribution, kind: "mixed" } })), "Sam (Teams estimate)");
  assert.equal(sourceSpeakerLabel(segment), "Anderen 2");
});

test("interface speaker names switch while source prefixes remain stable", () => {
  const label = speakerUiLabel(others);
  setInterfaceLanguage("nl");
  assert.equal(speakerLabel(others), "Anderen 2");
  assert.equal(String(label), "Anderen 2");
  assert.equal(sourceSpeakerLabel(others), "Anderen 2");
  setInterfaceLanguage("en");
  assert.equal(speakerLabel(others), "Speaker 2");
  assert.equal(String(label), "Speaker 2");
  assert.equal(sourceSpeakerLabel(others), "Anderen 2");
  assert.equal(speakerLabel({ speaker: "me", voice: null }), "Me");
  assert.equal(sourceSpeakerLabel({ speaker: "me", voice: null }), "Ik");
});

test("remembered speaker names remain verbatim in both interfaces", () => {
  const detail = { speakers: [{ voice: 2, name: "Samenvatting {number}" }] };
  for (const language of ["nl", "en"]) {
    setInterfaceLanguage(language);
    assert.equal(speakerLabel(others, detail), "Samenvatting {number}");
    assert.equal(String(speakerUiLabel(others, detail)), "Samenvatting {number}");
    assert.equal(sourceSpeakerLabel(others, detail), "Samenvatting {number}");
  }
});

test("rich attribution never turns a physical microphone into a person", () => {
  for (const [language, unknown, mixed, overlap] of [["en", "Unknown speaker", "Mixed speakers", "Overlapping speakers"], ["nl", "Onbekende spreker", "Meerdere sprekers", "Overlappende sprekers"]]) {
    setInterfaceLanguage(language);
    const microphone = { speaker: "me", voice: null, source_id: "microphone", attribution: { kind: "unknown", voices: [] } };
    assert.equal(speakerLabel(microphone), unknown);
    assert.equal(String(speakerUiLabel(microphone)), unknown);
    assert.equal(speakerLabel({ ...microphone, attribution: { kind: "mixed", voices: [1, 2] } }), mixed);
    assert.equal(speakerLabel({ ...microphone, attribution: { kind: "overlap", voices: [1, 2] } }), overlap);
    assert.equal(speakerLabel({ ...microphone, attribution: { kind: "single", voices: [2] } }, { speakers: [{ voice: 2, name: "Noor" }] }), "Noor");
    assert.equal(speakerLabel({ ...microphone, confirmed_speaker_name: "Samenvatting {number}" }), "Samenvatting {number}");
    assert.equal(String(speakerUiLabel({ ...microphone, confirmed_speaker_name: "Samenvatting {number}" })), "Samenvatting {number}");
    const self = { ...microphone, speaker: "others", source_id: "system_audio", attribution: { kind: "single", voices: [], self_voice: true } };
    assert.equal(speakerLabel(self), language === "en" ? "Me" : "Ik");
    assert.equal(speakerLabel({ ...self, confirmed_speaker_name: "Noor" }), "Noor");
    assert.equal(speakerLabel({ ...self, attribution: { kind: "unknown", voices: [], self_voice: true } }), unknown);
  }
  setInterfaceLanguage("en");
});

test("only uncustomized built-in template names are translated", () => {
  const original = { id: "algemeen", name: "Algemeen", builtin: true, customized: false };
  const label = templateUiLabel(original);
  setInterfaceLanguage("nl");
  assert.equal(templateLabel(original), "Algemeen");
  assert.equal(String(label), "Algemeen");
  setInterfaceLanguage("en");
  assert.equal(templateLabel(original), "General");
  assert.equal(String(label), "General");
  assert.equal(templateLabel({ ...original, customized: true }), "Algemeen");
  assert.equal(String(templateUiLabel({ ...original, customized: true })), "Algemeen");
  assert.equal(templateLabel({ ...original, builtin: false }), "Algemeen");
});

test("meeting dates and relative groups use the interface locale", () => {
  const now = new Date("2026-10-26T12:00:00Z");
  setInterfaceLanguage("nl");
  assert.equal(dayGroup("2026-10-26T12:00:00Z", now), "Vandaag");
  assert.equal(dayGroup("2026-10-23T12:00:00Z", now), "Deze week");
  assert.equal(dayGroup("2026-10-01T12:00:00Z", now), "Eerder");
  const dutch = meetingDate("2026-10-26T12:00:00Z");
  setInterfaceLanguage("en");
  assert.equal(dayGroup("2026-10-26T12:00:00Z", now), "Today");
  assert.equal(dayGroup("2026-10-23T12:00:00Z", now), "This week");
  assert.equal(dayGroup("2026-10-01T12:00:00Z", now), "Earlier");
  assert.notEqual(meetingDate("2026-10-26T12:00:00Z"), dutch);
  assert.equal(meetingDate("not a date"), "not a date");
});
