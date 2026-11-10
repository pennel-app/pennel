use std::path::{Path, PathBuf};

use crate::error::PennelError;

pub const FALLBACK_ID: &str = "algemeen";
pub const DEFAULT_SECTIONS: [&str; 3] = ["Summary", "Decisions", "Action items"];
const ID_MAX: usize = 40;

const BUILTINS: &[(&str, &str, &str)] = &[
    (
        "algemeen",
        "General",
        "Focus on what was discussed and agreed.",
    ),
    (
        "een-op-een",
        "One-to-one",
        "This is a 1-on-1. Include feedback, wellbeing and personal follow-ups.",
    ),
    (
        "klantgesprek",
        "Client meeting",
        "This is a client meeting. Capture the client's needs, questions, commitments made to them and next steps.",
    ),
    (
        "standup",
        "Standup",
        "This is a standup. Per person: done, next, blockers.",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct MeetingTemplate {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub customized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSpec {
    pub name: String,
    pub instruction: String,
    pub sections: Vec<String>,
    pub(crate) default_sections: bool,
}

impl TemplateSpec {
    /// Parses a template body: `## ` lines are sections, everything else is the instruction.
    pub fn parse(name: &str, body: &str) -> Self {
        let mut sections = Vec::new();
        let mut instruction = Vec::new();
        for line in body.lines() {
            match line.trim().strip_prefix("## ") {
                Some(section) if !section.trim().is_empty() => {
                    sections.push(section.trim().to_owned())
                }
                _ => instruction.push(line),
            }
        }
        let default_sections = sections.is_empty();
        if default_sections {
            sections = DEFAULT_SECTIONS.iter().map(|s| s.to_string()).collect();
        }
        Self {
            name: name.to_owned(),
            instruction: instruction.join("\n").trim().to_owned(),
            sections,
            default_sections,
        }
    }
}

fn builtin_body(instruction: &str) -> String {
    let sections: String = DEFAULT_SECTIONS
        .iter()
        .map(|s| format!("## {s}\n"))
        .collect();
    format!("{instruction}\n\n{sections}")
}

fn render(name: &str, body: &str) -> String {
    format!(
        "---\nname: {}\n---\n{}\n",
        name.replace('\n', " ").trim(),
        body.trim()
    )
}

/// Returns `(name, body)`. Only a leading block of `key: value` lines with a `name:` counts as frontmatter,
/// so a template that opens with a `---` rule keeps its text; without a usable name the file stem is used.
fn split_frontmatter(text: &str, stem: &str) -> (String, String) {
    let text = text.replace("\r\n", "\n");
    if let Some(rest) = text.strip_prefix("---\n")
        && let Some(end) = rest.find("\n---")
    {
        let block = &rest[..end];
        let is_frontmatter = block
            .lines()
            .all(|l| l.trim().is_empty() || l.contains(':'))
            && block.lines().any(|l| l.starts_with("name:"));
        if is_frontmatter {
            let name = block
                .lines()
                .find_map(|l| l.strip_prefix("name:"))
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| stem.to_owned());
            let body = rest[end + 4..].trim_start_matches('\n').to_owned();
            return (name, body);
        }
    }
    (stem.to_owned(), text)
}

pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(ID_MAX).collect();
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.is_empty() {
        "template".to_owned()
    } else {
        slug
    }
}

pub struct TemplateStore {
    dir: PathBuf,
}

impl TemplateStore {
    pub fn new(meetings_dir: &Path) -> Self {
        Self {
            dir: meetings_dir.join(".templates"),
        }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.md"))
    }

    fn read(&self, id: &str) -> Option<(String, String)> {
        if !self.custom_ids().iter().any(|c| c == id) {
            return None;
        }
        let text = std::fs::read_to_string(self.path(id)).ok()?;
        Some(split_frontmatter(&text, id))
    }

    fn custom_ids(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect()
    }

    /// Only built-in ids and the stems of files already in the folder are valid, so an id from the UI can
    /// never point outside `.templates`.
    fn is_known(&self, id: &str) -> bool {
        Self::builtin(id).is_some() || self.custom_ids().iter().any(|c| c == id)
    }

    fn builtin(id: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
        BUILTINS.iter().find(|(b, _, _)| *b == id)
    }

    pub fn list(&self) -> Vec<MeetingTemplate> {
        let custom = self.custom_ids();
        let mut list: Vec<MeetingTemplate> = BUILTINS
            .iter()
            .map(|(id, name, _)| {
                let file = self.read(id);
                MeetingTemplate {
                    id: (*id).into(),
                    name: file.map_or_else(|| (*name).into(), |(n, _)| n),
                    builtin: true,
                    customized: custom.iter().any(|c| c == id),
                }
            })
            .collect();
        let mut own: Vec<MeetingTemplate> = custom
            .iter()
            .filter(|id| Self::builtin(id).is_none())
            .filter_map(|id| {
                let (name, _) = self.read(id)?;
                Some(MeetingTemplate {
                    id: id.clone(),
                    name,
                    builtin: false,
                    customized: false,
                })
            })
            .collect();
        own.sort_by_key(|t| t.name.to_lowercase());
        list.extend(own);
        list
    }

    pub fn spec(&self, id: &str) -> TemplateSpec {
        self.resolved_spec(id).1
    }

    pub(crate) fn resolved_spec(&self, id: &str) -> (String, TemplateSpec) {
        if let Some((name, body)) = self.read(id) {
            return (id.into(), TemplateSpec::parse(&name, &body));
        }
        let (id, name, instruction) = Self::builtin(id)
            .or_else(|| Self::builtin(FALLBACK_ID))
            .expect("fallback exists");
        ((*id).into(), TemplateSpec::parse(name, instruction))
    }

    /// Body shown in the editor, without frontmatter.
    pub fn source(&self, id: &str) -> Option<String> {
        self.read(id)
            .map(|(_, body)| body)
            .or_else(|| Self::builtin(id).map(|(_, _, instruction)| builtin_body(instruction)))
    }

    pub fn save(&self, id: Option<&str>, name: &str, body: &str) -> Result<String, PennelError> {
        std::fs::create_dir_all(&self.dir).map_err(PennelError::storage)?;
        let id = match id {
            Some(id) if self.is_known(id) => id.to_owned(),
            Some(_) => return Err(PennelError::NotFound),
            None => {
                let base = slug(name);
                std::iter::once(base.clone())
                    .chain((2..).map(|n| format!("{base}-{n}")))
                    .find(|candidate| {
                        Self::builtin(candidate).is_none() && !self.path(candidate).exists()
                    })
                    .expect("unbounded iterator")
            }
        };
        let tmp = self.dir.join(format!(".{id}.tmp"));
        std::fs::write(&tmp, render(name, body)).map_err(PennelError::storage)?;
        std::fs::rename(&tmp, self.path(&id)).map_err(PennelError::storage)?;
        Ok(id)
    }

    /// Removes a custom template, or the override of a built-in (restoring the original).
    pub fn delete(&self, id: &str) -> Result<(), PennelError> {
        if !self.custom_ids().iter().any(|c| c == id) {
            return Err(PennelError::NotFound);
        }
        match std::fs::remove_file(self.path(id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(PennelError::NotFound),
            Err(e) => Err(PennelError::storage(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, TemplateStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TemplateStore::new(dir.path());
        (dir, store)
    }

    fn ids(store: &TemplateStore) -> Vec<String> {
        store.list().into_iter().map(|t| t.id).collect()
    }

    #[test]
    fn builtins_listed_first() {
        let (_dir, store) = store();
        store.save(None, "Retro", "x").unwrap();
        store.save(None, "Andere", "y").unwrap();
        assert_eq!(
            ids(&store),
            [
                "algemeen",
                "een-op-een",
                "klantgesprek",
                "standup",
                "andere",
                "retro"
            ]
        );
        assert!(store.list()[0].builtin);
        assert!(!store.list()[4].builtin);
    }

    #[test]
    fn builtin_sources_are_english_without_changing_persisted_ids() {
        let (_dir, store) = store();
        let names: Vec<_> = store
            .list()
            .into_iter()
            .map(|template| (template.id, template.name))
            .collect();
        assert_eq!(
            names,
            [
                ("algemeen", "General"),
                ("een-op-een", "One-to-one"),
                ("klantgesprek", "Client meeting"),
                ("standup", "Standup")
            ]
            .map(|(id, name)| (id.to_owned(), name.to_owned()))
        );
        assert!(store.spec("algemeen").default_sections);
        assert_eq!(
            store.spec("algemeen").sections,
            ["Summary", "Decisions", "Action items"]
        );
        assert!(store.source("algemeen").unwrap().contains("## Summary"));
    }

    #[test]
    fn heading_provenance_preserves_authored_and_customized_templates() {
        let (_dir, store) = store();
        let headingless = store
            .save(None, "Mijn sjabloon", "Behoud mijn tekst.")
            .unwrap();
        assert!(store.spec(&headingless).default_sections);
        let body = store.source("algemeen").unwrap();
        store.save(Some("algemeen"), "Algemeen", &body).unwrap();
        assert!(!store.spec("algemeen").default_sections);
        assert_eq!(store.list()[0].name, "Algemeen");
        assert!(!TemplateSpec::parse("Custom", "## Summary\n## Actiepunten").default_sections);
    }

    #[test]
    fn custom_sections_drive_spec() {
        let (_dir, store) = store();
        let id = store
            .save(
                None,
                "Retro",
                "Wat ging goed en wat beter kan.\n\n## Goed\n## Beter\n## Actiepunten",
            )
            .unwrap();
        let spec = store.spec(&id);
        assert_eq!(spec.name, "Retro");
        assert_eq!(spec.instruction, "Wat ging goed en wat beter kan.");
        assert_eq!(spec.sections, ["Goed", "Beter", "Actiepunten"]);
        assert_eq!(
            store.source(&id).unwrap(),
            "Wat ging goed en wat beter kan.\n\n## Goed\n## Beter\n## Actiepunten\n"
        );
    }

    #[test]
    fn default_sections_without_headings() {
        assert_eq!(
            TemplateSpec::parse("x", "Kort en krachtig.").sections,
            DEFAULT_SECTIONS
        );
    }

    #[test]
    fn override_builtin() {
        let (_dir, store) = store();
        assert_eq!(
            store
                .save(
                    Some("standup"),
                    "Daily",
                    "Per persoon.\n## Gisteren\n## Vandaag"
                )
                .unwrap(),
            "standup"
        );
        let standup = store
            .list()
            .into_iter()
            .find(|t| t.id == "standup")
            .unwrap();
        assert!(standup.builtin && standup.customized);
        assert_eq!(standup.name, "Daily");
        assert_eq!(store.spec("standup").sections, ["Gisteren", "Vandaag"]);
        store.delete("standup").unwrap();
        assert_eq!(store.spec("standup").sections, DEFAULT_SECTIONS);
    }

    #[test]
    fn file_without_frontmatter_uses_stem() {
        let (dir, store) = store();
        std::fs::create_dir_all(dir.path().join(".templates")).unwrap();
        std::fs::write(
            dir.path().join(".templates/sales.md"),
            "Focus op deals.\r\n## Deals\r\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".templates/leeg.md"),
            "---\nname:\n---\nIets",
        )
        .unwrap();
        let list = store.list();
        assert!(list.iter().any(|t| t.id == "sales" && t.name == "sales"));
        assert!(list.iter().any(|t| t.id == "leeg" && t.name == "leeg"));
        assert_eq!(store.spec("sales").sections, ["Deals"]);
    }

    #[test]
    fn missing_template_falls_back() {
        let (_dir, store) = store();
        let spec = store.spec("bestaat-niet");
        assert_eq!(spec.name, "General");
        assert_eq!(spec.instruction, "Focus on what was discussed and agreed.");
    }

    #[test]
    fn slug_is_safe_and_unique() {
        let (_dir, store) = store();
        assert_eq!(store.save(None, "Retro / Q4: 🚀", "x").unwrap(), "retro-q4");
        assert_eq!(
            store.save(None, "Retro / Q4: 🚀", "x").unwrap(),
            "retro-q4-2"
        );
        assert_eq!(store.save(None, "🚀", "x").unwrap(), "template");
        assert_eq!(store.save(None, "Algemeen", "x").unwrap(), "algemeen-2");
    }

    #[test]
    fn ids_outside_the_folder_are_rejected() {
        let (dir, store) = store();
        std::fs::write(dir.path().join("geheim.md"), "x").unwrap();
        assert_eq!(store.delete("../geheim"), Err(PennelError::NotFound));
        assert!(dir.path().join("geheim.md").exists());
        assert_eq!(store.source("../geheim"), None);
        assert_eq!(
            store.save(Some("../geheim"), "x", "y"),
            Err(PennelError::NotFound)
        );
    }

    #[test]
    fn hand_made_file_is_saved_in_place() {
        let (dir, store) = store();
        std::fs::create_dir_all(dir.path().join(".templates")).unwrap();
        std::fs::write(dir.path().join(".templates/Retro_v2.md"), "oud").unwrap();
        assert_eq!(
            store.save(Some("Retro_v2"), "Retro v2", "nieuw").unwrap(),
            "Retro_v2"
        );
        assert_eq!(store.list().iter().filter(|t| !t.builtin).count(), 1);
        assert_eq!(store.source("Retro_v2").unwrap(), "nieuw\n");
    }

    #[test]
    fn names_round_trip_and_rules_stay_body() {
        let (dir, store) = store();
        let id = store.save(None, "\"Goed\": wat werkt", "Tekst").unwrap();
        assert_eq!(
            store.list().iter().find(|t| t.id == id).unwrap().name,
            "\"Goed\": wat werkt"
        );
        std::fs::write(
            dir.path().join(".templates/regel.md"),
            "---\nGeen frontmatter\n---\n## A",
        )
        .unwrap();
        let (name, body) = store.read("regel").unwrap();
        assert_eq!(name, "regel");
        assert!(body.starts_with("---\nGeen frontmatter"));
    }

    #[test]
    fn builtin_cannot_be_deleted() {
        let (_dir, store) = store();
        assert_eq!(store.delete("algemeen"), Err(PennelError::NotFound));
        assert!(ids(&store).contains(&"algemeen".to_string()));
    }
}
