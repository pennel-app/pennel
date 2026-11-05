export const questionStarterIds = ["decisions", "actions", "questions"] as const;
export type QuestionStarterId = typeof questionStarterIds[number];

const questions = {
  en: {
    decisions: "Which decisions were made?",
    actions: "What are the next steps?",
    questions: "Which questions remain unanswered?",
  },
  nl: {
    decisions: "Welke besluiten zijn er genomen?",
    actions: "Wat zijn de volgende stappen?",
    questions: "Welke vragen zijn nog onbeantwoord?",
  },
};

export function questionStarterText(id: QuestionStarterId, contentLanguage: string | null): string {
  const language = contentLanguage?.toLowerCase().split("-")[0] === "nl" ? "nl" : "en";
  return questions[language][id];
}
