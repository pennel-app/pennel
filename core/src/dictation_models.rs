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
        if !settings.dictation_models.contains(&settings.dictation_stt) {
            settings
                .dictation_models
                .push(settings.dictation_stt.clone());
        }
        settings.dictation_stt = choice;
        self.persist_settings_locked(settings)?;
        Ok((selected, account))
    }

    fn ready_dictation_models(
        &self,
        settings: &Settings,
        keys: &ApiKeys,
    ) -> Vec<ConfiguredDictationModel> {
        let local = if crate::cpu_support::local_models_unavailable_reason().is_none() {
            self.local_models(settings.language.clone(), true)
        } else {
            Vec::new()
        };
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
                        if model != "dictation"
                            || !self
                                .system_speech_available
                                .load(std::sync::atomic::Ordering::SeqCst)
                            || self.system_stt.read().unwrap().is_none()
                        {
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
        if !settings.dictation_models.contains(&settings.dictation_stt) {
            settings
                .dictation_models
                .push(settings.dictation_stt.clone());
        }
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
    fn switching_materializes_the_current_choice_omitted_from_a_partial_saved_list() {
        let (_directory, engine) = engine();
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "server".into(),
            kind: ProviderKind::Custom,
            name: "Server".into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
        });
        settings.dictation_models = vec![choice("server", "B")];
        settings.dictation_stt = choice("server", "A");
        engine.update_settings(settings.clone()).unwrap();
        assert_eq!(engine.configured_dictation_models(keys()).len(), 2);
        engine
            .select_dictation_model(choice("server", "B"), keys())
            .unwrap();
        assert_eq!(engine.configured_dictation_models(keys()).len(), 2);
        assert_eq!(
            engine.next_dictation_model(keys()).unwrap().choice,
            choice("server", "A")
        );
        engine.update_settings(settings).unwrap();
        assert_eq!(
            engine.next_dictation_model(keys()).unwrap().choice,
            choice("server", "B")
        );
        assert_eq!(
            engine.next_dictation_model(keys()).unwrap().choice,
            choice("server", "A")
        );
    }

    #[cfg(not(feature = "local-models"))]
    #[test]
    fn a_local_file_does_not_imply_a_compiled_recognizer() {
        let (directory, engine) = engine();
        let model = "whisper-small-q5_1";
        std::fs::create_dir_all(directory.path().join("models")).unwrap();
        std::fs::write(
            directory.path().join("models").join(format!("{model}.bin")),
            b"synthetic model file; never inferred",
        )
        .unwrap();
        assert_eq!(engine.model_state(model.into()), ModelState::Downloaded);
        let mut settings = engine.settings();
        settings.dictation_stt = choice("local", model);
        engine.update_settings(settings).unwrap();
        assert!(engine.configured_dictation_models(keys()).is_empty());
    }

    #[test]
    fn host_speech_readiness_filters_unsupported_or_denied_system_choices_without_inference() {
        struct Speech(Arc<std::sync::atomic::AtomicUsize>);
        impl crate::SystemTranscriber for Speech {
            fn transcribe(
                &self,
                _: Vec<f32>,
                _: u32,
                _: Option<String>,
                _: Vec<String>,
            ) -> Result<String, PennelError> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                panic!("readiness must not infer");
            }
        }
        let (_directory, engine) = engine();
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "apple".into(),
            kind: ProviderKind::AppleSpeech,
            name: "Apple dictation".into(),
            base_url: String::new(),
        });
        settings.dictation_stt = choice("apple", "dictation");
        engine.update_settings(settings).unwrap();
        engine.set_system_speech_available(true);
        assert!(engine.configured_dictation_models(keys()).is_empty());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        engine.set_system_transcriber(Arc::new(Speech(calls.clone())));
        assert_eq!(engine.configured_dictation_models(keys()).len(), 1);
        engine.set_system_speech_available(false);
        assert!(engine.configured_dictation_models(keys()).is_empty());
        assert!(
            engine
                .select_dictation_model(choice("apple", "dictation"), keys())
                .is_err()
        );
        assert_eq!(
            engine.settings().dictation_stt,
            choice("apple", "dictation")
        );
        engine.set_system_speech_available(true);
        assert_eq!(engine.configured_dictation_models(keys()).len(), 1);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(
            serde_json::to_value(engine.settings())
                .unwrap()
                .get("system_speech_available")
                .is_none()
        );
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
