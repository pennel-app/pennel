use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Record)]
pub struct ConfiguredDictationModel {
    pub choice: ModelChoice,
    pub label: String,
}

impl PennelEngine {
    pub(crate) fn select_dictation_model_snapshot(
        &self,
        choice: ModelChoice,
        keys: ApiKeys,
    ) -> Result<(ConfiguredDictationModel, ProviderAccount), PennelError> {
        let _gate = self.meeting_gate.lock().unwrap();
        let mut settings = self.settings();
        let selected = self
            .ready_dictation_models(&settings, &keys)
            .into_iter()
            .find(|entry| entry.choice == choice)
            .ok_or(PennelError::NotFound)?;
        let (account, _) = settings.resolve(&choice).ok_or(PennelError::NotFound)?;
        settings.dictation_stt = choice;
        self.persist_settings_locked(settings)?;
        Ok((selected, account))
    }

    fn ready_dictation_models(
        &self,
        settings: &Settings,
        keys: &ApiKeys,
    ) -> Vec<ConfiguredDictationModel> {
        let local = self.models.local_models(settings.language.as_deref(), true);
        let mut seen = HashSet::new();
        settings
            .dictation_models
            .iter()
            .chain(std::iter::once(&settings.dictation_stt))
            .filter(|choice| seen.insert((choice.provider_id.clone(), choice.model.clone())))
            .filter_map(|choice| {
                let (account, model) = settings.resolve(choice)?;
                if model.trim().is_empty()
                    || !supports(account.kind, ProviderTask::DictationStt)
                    || requires_key(account.kind) && keys.get(&account).is_none()
                {
                    return None;
                }
                let name = match account.kind {
                    ProviderKind::Local => {
                        if self.models.state(&model) != ModelState::Downloaded {
                            return None;
                        }
                        local.iter().find(|entry| entry.id == model)?.name.clone()
                    }
                    ProviderKind::AppleSpeech => {
                        if model != "dictation" || self.system_stt.read().unwrap().is_none() {
                            return None;
                        }
                        "Apple dictation".into()
                    }
                    ProviderKind::PennelCloud => {
                        if !self.cloud_signed_in() {
                            return None;
                        }
                        if model == "pennel-auto" {
                            "Pennel Cloud".into()
                        } else {
                            model.clone()
                        }
                    }
                    _ => {
                        let endpoint = account.base_url.trim();
                        if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
                            return None;
                        }
                        model.clone()
                    }
                };
                let provider = if account.name.trim().is_empty() {
                    account.kind.label()
                } else {
                    account.name.trim()
                };
                let label = if name == provider {
                    name
                } else {
                    format!("{name} · {provider}")
                };
                Some(ConfiguredDictationModel {
                    choice: choice.clone(),
                    label,
                })
            })
            .collect()
    }
}

#[uniffi::export]
impl PennelEngine {
    /// Configuration readiness only: no network calls, download, token refresh or inference.
    pub fn configured_dictation_models(&self, keys: ApiKeys) -> Vec<ConfiguredDictationModel> {
        self.ready_dictation_models(&self.settings(), &keys)
    }

    pub fn select_dictation_model(
        &self,
        choice: ModelChoice,
        keys: ApiKeys,
    ) -> Result<ConfiguredDictationModel, PennelError> {
        self.select_dictation_model_snapshot(choice, keys)
            .map(|(selected, _)| selected)
    }

    pub fn next_dictation_model(
        &self,
        keys: ApiKeys,
    ) -> Result<ConfiguredDictationModel, PennelError> {
        let _gate = self.meeting_gate.lock().unwrap();
        let mut settings = self.settings();
        let models = self.ready_dictation_models(&settings, &keys);
        if models.len() < 2 {
            return Err(PennelError::NotFound);
        }
        let index = models
            .iter()
            .position(|entry| entry.choice == settings.dictation_stt)
            .map_or(0, |index| (index + 1) % models.len());
        let selected = models[index].clone();
        settings.dictation_stt = selected.choice.clone();
        self.persist_settings_locked(settings)?;
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> (tempfile::TempDir, Arc<PennelEngine>) {
        let directory = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(directory.path().to_string_lossy().into_owned()).unwrap();
        (directory, engine)
    }
    fn choice(provider: &str, model: &str) -> ModelChoice {
        ModelChoice {
            provider_id: provider.into(),
            model: model.into(),
        }
    }
    fn keys() -> ApiKeys {
        ApiKeys {
            by_provider: HashMap::new(),
        }
    }

    #[test]
    fn configured_models_keep_only_explicit_supported_ready_choices_in_saved_order() {
        let (_directory, engine) = engine();
        let mut settings = engine.settings();
        settings.providers.extend([
            ProviderAccount {
                id: "server".into(),
                kind: ProviderKind::Custom,
                name: "Work server".into(),
                base_url: "http://127.0.0.1:1234/v1".into(),
            },
            ProviderAccount {
                id: "agent".into(),
                kind: ProviderKind::Codex,
                name: "Codex".into(),
                base_url: String::new(),
            },
        ]);
        settings.dictation_stt = choice("server", "alpha");
        settings.dictation_models = vec![
            choice("groq", "whisper"),
            choice("server", "beta"),
            choice("server", "beta"),
            choice("agent", "model"),
            choice("local", "missing"),
            choice("absent", "model"),
            choice("server", "alpha"),
        ];
        engine.update_settings(settings).unwrap();
        let models = engine.configured_dictation_models(keys());
        assert_eq!(
            models
                .iter()
                .map(|row| row.choice.clone())
                .collect::<Vec<_>>(),
            vec![choice("server", "beta"), choice("server", "alpha")]
        );
        assert_eq!(models[0].label, "beta · Work server");
        assert_eq!(
            engine.next_dictation_model(keys()).unwrap().choice,
            choice("server", "beta")
        );
        assert_eq!(
            engine.next_dictation_model(keys()).unwrap().choice,
            choice("server", "alpha")
        );
        let mut with_key = keys();
        with_key
            .by_provider
            .insert("groq".into(), "synthetic-key".into());
        assert_eq!(engine.configured_dictation_models(with_key).len(), 3);
    }

    #[test]
    fn upgrade_keeps_the_single_saved_choice_and_never_implies_readiness() {
        let (_directory, engine) = engine();
        let mut value = serde_json::to_value(engine.settings()).unwrap();
        value.as_object_mut().unwrap().remove("dictation_models");
        let settings: Settings = serde_json::from_value(value).unwrap();
        assert!(settings.dictation_models.is_empty());
        assert!(engine.configured_dictation_models(keys()).is_empty());
        assert!(engine.next_dictation_model(keys()).is_err());
    }

    #[test]
    fn selecting_an_unconfigured_or_unready_model_does_not_change_the_saved_choice() {
        let (_directory, engine) = engine();
        let before = engine.settings();
        assert!(
            engine
                .select_dictation_model(choice("groq", "not-configured"), keys())
                .is_err()
        );
        assert_eq!(engine.settings(), before);
    }
}
