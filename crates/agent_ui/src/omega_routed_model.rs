//! The routed decision. `OMEGA-DELTA-0202`.
//!
//! One answer to "which model is serving this thread", and every label a person
//! reads is a function of it: the tier control's face and the executor
//! disclosure line. `OMEGA-DELTA-0208` folded what used to be a separate
//! composer status line into that one line, because the two were the same fact
//! said twice — see [`chrome_line`].
//!
//! # Why this exists
//!
//! The owner opened a pre-existing thread. The tier control said **Luna**. The
//! disclosure line under it said `openagents/kimi-k3`, the status line agreed
//! with the disclosure, and Kimi is what answered. Two surfaces, one turn, two
//! different models named — which is precisely the defect class
//! `OMEGA-DELTA-0131` was written for, restated on a new control.
//!
//! The cause was two sources of truth, exactly as it was then. The disclosure
//! read the thread's own model. The tier control read
//! [`crate::omega_model_tier::selected`] — a process-wide static that is *the
//! standing choice for the next connection* and resets to Luna at every launch.
//! A control whose face comes from a different place than the work cannot be
//! kept in agreement by care; it is only ever accidentally right.
//!
//! # The shape of the fix
//!
//! [`RoutedModel`] is derived from [`ExecutorDisclosure`], which is already the
//! projection over the thread's durable model record — and, since
//! `OMEGA-DELTA-0202`, over the live `OMEGA-DELTA-0201` fallback rung when a
//! turn has fallen onto one. There is no second store to keep in step, and no
//! surface is permitted its own answer: a caller asks this module what is
//! serving the thread and renders that.
//!
//! The standing choice is not deleted — it still decides what a *new*
//! conversation starts on. It is simply no longer allowed to describe a thread
//! that has already resolved a model.

use gpui::App;
use language_model::LanguageModelRegistry;
use omega_front_door::ExecutorDisclosure;
use ui::SharedString;

use crate::omega_model_tier::{ModelTier, RoutedFace};

/// The model that is actually serving a thread, as a provider/model pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedModel {
    provider_id: String,
    model_id: String,
}

impl RoutedModel {
    #[must_use]
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
        }
    }

    /// The routed decision a thread's disclosure record already holds.
    ///
    /// `None` when the executor did not disclose a model — an external ACP
    /// agent that advertises no model config, or a thread whose model has not
    /// resolved yet. Saying "not disclosed" is the honest answer there, and it
    /// is the same answer [`ExecutorDisclosure::label`] gives, so the surfaces
    /// stay in agreement even about their ignorance.
    #[must_use]
    pub fn from_disclosure(disclosure: &ExecutorDisclosure) -> Option<Self> {
        let provider_id = disclosure.provider.as_deref()?;
        let model_id = disclosure.model.as_deref()?;
        if provider_id.is_empty() || model_id.is_empty() {
            return None;
        }
        Some(Self::new(provider_id, model_id))
    }

    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// `provider/model` — the pair every surface is derived from.
    #[must_use]
    pub fn wire_id(&self) -> String {
        format!("{}/{}", self.provider_id, self.model_id)
    }

    /// The Omega tier this pair is, when it is one of the three.
    #[must_use]
    pub fn tier(&self) -> Option<ModelTier> {
        ModelTier::for_model(&self.provider_id, &self.model_id)
    }

    /// What the tier control shows for this thread.
    #[must_use]
    pub fn face(&self) -> RoutedFace {
        RoutedFace::for_model(&self.provider_id, &self.model_id)
    }

    /// The model's own name, as a person says it — `Kimi K3`, not
    /// `openagents/kimi-k3`.
    ///
    /// `OMEGA-DELTA-0202` introduced this as `status_line`, when it was the
    /// whole of a second line under the disclosure. `OMEGA-DELTA-0208` folded
    /// that line into the disclosure's own, so this is now the model segment of
    /// one line rather than a line, and it is named for what it is. The
    /// constraint it was born with is unchanged and is why the fold is safe:
    /// **at most the model name**. It used to carry the route receipt —
    /// dispatch reference, route summary, override mode, route-fallback state —
    /// none of which is a fact about the person's work, and the owner's
    /// standing no-exposition law applies to it. The route receipt is still
    /// recorded; it is simply not composer copy.
    #[must_use]
    pub fn human_name(&self) -> SharedString {
        self.face().model_name
    }
}

/// The one line a person's chrome shows for a thread: who ran it, and the model
/// under the name that model actually has.
///
/// `OMEGA-DELTA-0208`. The composer drew two lines from one fact — the record's
/// own [`ExecutorDisclosure::label`], which names the model by its wire pair,
/// and beneath it [`RoutedModel::human_name`], which names the same model as a
/// person says it. So a thread on Kimi read `Omega Agent · openagents/kimi-k3`
/// above `Kimi K3`. The owner: "remove the `openagents/gpt-5.6-luna` … its
/// duplicative with gpt 5.6 luna like the real name."
///
/// One line, one name. `openagents/kimi-k3` is a wire identifier, and the
/// standing no-exposition law is the same law that took the class token off
/// this row in omega#100: a person is not here to learn Omega's routing
/// vocabulary. The pair is not lost — [`ExecutorDisclosure::label`] still
/// renders it for receipts, copied system specs and machine readers, which is
/// where an exact identifier is the useful answer.
///
/// Everything else about the line is unchanged, because the shape stays in
/// [`ExecutorDisclosure::label_with_model`]: the run reference and the fallback
/// clause are still said. When an ACP executor supplies a model but no provider,
/// the human-facing line uses the model name by itself; the record retains the
/// exact provider-disclosure state for receipts and machine readers.
#[must_use]
pub fn chrome_line(disclosure: &ExecutorDisclosure) -> String {
    let model = RoutedModel::from_disclosure(disclosure).map_or_else(
        || disclosure.model_phrase(),
        |routed| routed.human_name().to_string(),
    );
    disclosure.label_with_model(&model)
}

/// The face for a thread, falling back to the standing choice only when nothing
/// has been routed yet.
///
/// Prefer [`face_for_next_turn`]. This entry point cannot see the registry, so
/// its `None` arm can only report the standing choice — which is what
/// `OMEGA-DELTA-0207` found naming a model the send did not use.
#[must_use]
pub fn face_for(routed: Option<&RoutedModel>, standing: ModelTier) -> RoutedFace {
    match routed {
        Some(routed) => routed.face(),
        None => RoutedFace::pending(standing),
    }
}

/// The pair a thread that has not resolved a model yet will actually start on.
///
/// `OMEGA-DELTA-0207`. This asks the same question `Thread::send_existing`
/// answers, before the thread exists: the registry's default model is what
/// `NativeAgent` hands a new thread and what `Thread::ensure_model` fills an
/// unset one with. Reading it here is what makes the pre-session label a
/// statement about the next turn rather than a guess.
#[must_use]
pub fn pending_routed_model(cx: &App) -> Option<RoutedModel> {
    let registry = LanguageModelRegistry::try_global(cx)?;
    let configured = registry.read(cx).default_model()?;
    Some(RoutedModel::new(
        configured.model.provider_id().0.to_string(),
        configured.model.id().0.to_string(),
    ))
}

/// The face every composer shows: the routed decision when there is one, and
/// otherwise the model the next turn will actually start on.
///
/// `OMEGA-DELTA-0207`. The standing choice is the last resort and nothing
/// else. It is a process-wide static that begins every launch at `Luna` and is
/// never seeded from settings, so a composer that read it named **Luna** on a
/// thread whose send went to `openagents/gemini-3.6-flash`. It survives here
/// only for a process with no registry at all — a test harness, or a window
/// drawn before the providers install — where there is no better answer and no
/// send to disagree with yet.
#[must_use]
pub fn face_for_next_turn(
    routed: Option<&RoutedModel>,
    standing: ModelTier,
    cx: &App,
) -> RoutedFace {
    if let Some(routed) = routed {
        return routed.face();
    }
    match pending_routed_model(cx) {
        Some(pending) => pending.face(),
        None => RoutedFace::pending(standing),
    }
}

#[cfg(test)]
mod tests {
    use omega_front_door::ExecutorClass;

    use super::*;

    fn native_disclosure(provider: &str, model: &str) -> ExecutorDisclosure {
        ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            agent_id: "Omega Agent".to_owned(),
            provider: Some(provider.to_owned()),
            model: Some(model.to_owned()),
            run_ref: None,
            route: None,
        }
    }

    /// The live defect, as a test. `OMEGA-DELTA-0202`.
    ///
    /// The stored default is Luna and the routed decision is Kimi K3. Every
    /// surface must report Kimi: the tier control's face, the disclosure line,
    /// and the composer status line. Before this fix the first of the three read
    /// the standing choice and said **Luna** while the other two said
    /// `openagents/kimi-k3`.
    #[test]
    fn every_surface_reports_the_routed_model_when_the_stored_default_differs() {
        crate::omega_model_tier::clear_selection_for_test();
        let stored_default = crate::omega_model_tier::selected();
        assert_eq!(
            stored_default,
            ModelTier::Luna,
            "the fixture requires the stored default to be Luna"
        );

        let disclosure = native_disclosure("openagents", "kimi-k3");
        let routed = RoutedModel::from_disclosure(&disclosure).expect("a native turn discloses");

        assert_eq!(routed.wire_id(), "openagents/kimi-k3");
        assert_eq!(routed.tier(), Some(ModelTier::Pro));

        let face = face_for(Some(&routed), stored_default);
        assert_eq!(face.tier, Some(ModelTier::Pro));
        assert_eq!(face.label.as_ref(), "Pro");
        assert_eq!(face.model_name.as_ref(), "Kimi K3");

        let status = routed.human_name();
        assert_eq!(status.as_ref(), "Kimi K3");

        let line = disclosure.label();
        assert!(
            line.contains("openagents/kimi-k3"),
            "the disclosure line must name the routed model: {line}"
        );

        for surface in [face.label.as_ref(), status.as_ref(), line.as_str()] {
            assert!(
                !surface.contains("Luna") && !surface.contains("gpt-5.6"),
                "no surface may name the stored default while another model \
                 serves the turn: {surface}"
            );
        }
        assert_eq!(
            ModelTier::Pro.model_id(),
            "kimi-k3",
            "the tier the face reports must be the routed model's own tier"
        );
    }

    /// The live defect, as a test. `OMEGA-DELTA-0208`.
    ///
    /// The owner read `Omega Agent · openagents/kimi-k3` with `Kimi K3` under
    /// it and said: "remove the `openagents/gpt-5.6-luna` … its duplicative
    /// with gpt 5.6 luna like the real name."
    ///
    /// So the chrome line names the model once, under the name the model has.
    /// The wire pair is not deleted — the record's own line still carries it,
    /// which is asserted here too, because "the id is gone from the chrome" and
    /// "the id is gone" are different claims and only the first one is wanted.
    #[test]
    fn the_chrome_line_names_one_model_once_and_never_by_its_wire_id() {
        for (provider, model, human) in [
            ("openagents", "kimi-k3", "Kimi K3"),
            ("openagents", "gpt-5.6-luna", "GPT-5.6 Luna"),
            ("openagents", "gemini-3.6-flash", "Gemini 3.6 Flash"),
        ] {
            let disclosure = native_disclosure(provider, model);
            let line = chrome_line(&disclosure);

            assert_eq!(
                line,
                format!("Omega Agent · {human}"),
                "the chrome line must name the executor and the model, once each"
            );
            let wire = format!("{provider}/{model}");
            assert!(
                !line.contains(&wire),
                "the wire pair is exposition on a person's chrome: {line}"
            );
            assert_eq!(
                line.matches(human).count(),
                1,
                "the model is named twice again: {line}"
            );

            // And the record still knows the exact pair, for the receipt, the
            // copied system spec, and every machine reader.
            assert!(
                disclosure.label().contains(&wire),
                "the record's own line must keep the exact identifier: {}",
                disclosure.label()
            );
        }
    }

    /// The fold changed which word names the model and nothing else.
    ///
    /// `OMEGA-DELTA-0208`. A run reference is still said, a fallback is still
    /// said, and a genuinely unknown model is still said to be undisclosed. A
    /// known model with an undisclosed provider is named without exposing the
    /// record-layer placeholder in application chrome.
    #[test]
    fn folding_the_line_kept_every_other_part_of_it() {
        let delegated = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-acp".to_owned(),
            provider: Some("openagents".to_owned()),
            model: Some("kimi-k3".to_owned()),
            run_ref: Some("operation.full-auto.77".to_owned()),
            route: None,
        };
        let line = chrome_line(&delegated);
        assert_eq!(line, "codex-acp · Kimi K3 · operation.full-auto.77");

        let fell_back = ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            agent_id: "Omega Agent".to_owned(),
            provider: Some("openagents".to_owned()),
            model: Some("gpt-5.6-luna".to_owned()),
            run_ref: None,
            route: Some(omega_front_door::RouteReason::EngineUnreachable),
        };
        let line = chrome_line(&fell_back);
        assert!(
            line.starts_with("Omega Agent · GPT-5.6 Luna · routed: "),
            "a fallback a person could not otherwise see must still be said: {line}"
        );

        let undisclosed = ExecutorDisclosure {
            class: ExecutorClass::ExternalAcp,
            agent_id: "codex".to_owned(),
            provider: None,
            model: None,
            run_ref: None,
            route: None,
        };
        assert_eq!(
            chrome_line(&undisclosed),
            undisclosed.label(),
            "with nothing to name humanly the chrome line is the record's own, \
             so ignorance is still stated rather than dropped"
        );

        let model_only = ExecutorDisclosure {
            class: ExecutorClass::ExternalAcp,
            agent_id: "codex-acp".to_owned(),
            provider: None,
            model: Some("GPT-5.6-Sol".to_owned()),
            run_ref: None,
            route: None,
        };
        assert_eq!(chrome_line(&model_only), "codex-acp · GPT-5.6-Sol");
        assert!(!chrome_line(&model_only).contains("provider not disclosed"));
    }

    /// The three surfaces are one function of one record, so they cannot
    /// disagree for any model at all — not only for the pair that was reported.
    #[test]
    fn no_disclosed_model_produces_two_different_answers() {
        for (provider, model) in [
            ("openagents", "gpt-5.6-luna"),
            ("openagents", "gemini-3.6-flash"),
            ("openagents", "kimi-k3"),
            ("anthropic", "claude-sonnet-4"),
            ("google", "gemini-3.6-flash"),
        ] {
            let disclosure = native_disclosure(provider, model);
            let routed =
                RoutedModel::from_disclosure(&disclosure).expect("both parts are disclosed");
            let face = face_for(Some(&routed), ModelTier::Luna);

            assert_eq!(routed.wire_id(), format!("{provider}/{model}"));
            assert_eq!(face.model_name, routed.human_name());
            match face.tier {
                Some(tier) => {
                    assert_eq!(tier.provider_id(), provider);
                    assert_eq!(tier.model_id(), model);
                }
                None => assert_eq!(face.label.as_ref(), model),
            }
            assert!(
                disclosure.label().contains(&routed.wire_id()),
                "the disclosure line is the same pair the face is derived from"
            );
        }
    }

    /// A thread with nothing routed yet is the one case the standing choice may
    /// describe, because it is then a statement about the next turn.
    #[test]
    fn an_unrouted_thread_falls_back_to_the_standing_choice() {
        let undisclosed = ExecutorDisclosure {
            class: ExecutorClass::ExternalAcp,
            agent_id: "codex".to_owned(),
            provider: None,
            model: None,
            run_ref: None,
            route: None,
        };
        assert_eq!(RoutedModel::from_disclosure(&undisclosed), None);
        let face = face_for(None, ModelTier::Flash);
        assert_eq!(face.tier, Some(ModelTier::Flash));
        assert_eq!(face.label.as_ref(), "Flash");
    }

    /// The live defect from the owner's run of `3becb7c004`. `OMEGA-DELTA-0207`.
    ///
    /// The configured default is Luna, the standing static is a stale Flash,
    /// and no thread has routed anything yet. The composer must name **Luna** —
    /// the model the send will actually dispatch on — rather than the standing
    /// choice. The owner saw the mirror image of this: a label reading Luna
    /// over a send that went to `openagents/gemini-3.6-flash`.
    #[gpui::test]
    fn a_stale_standing_choice_never_outranks_the_model_the_next_turn_starts_on(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let registry =
                install_registry_defaulting_to("openagents", "gpt-5.6-luna", "GPT-5.6 Luna", cx);
            drop(registry);

            // The stale standing choice: a different tier entirely.
            crate::omega_model_tier::select_for_test(ModelTier::Flash);
            assert_eq!(crate::omega_model_tier::selected(), ModelTier::Flash);

            // What the next turn will actually start on.
            let pending = pending_routed_model(cx).expect("the registry has a default model");
            assert_eq!(pending.wire_id(), "openagents/gpt-5.6-luna");

            // Every surface, from that one answer.
            let face = face_for_next_turn(None, crate::omega_model_tier::selected(), cx);
            assert_eq!(
                face.label.as_ref(),
                "Luna",
                "the tier control named the standing choice instead of the \
                 model the send would dispatch on"
            );
            assert_eq!(face.model_name.as_ref(), "GPT-5.6 Luna");
            assert_eq!(face.tier, Some(ModelTier::Luna));
            assert_eq!(pending.human_name().as_ref(), "GPT-5.6 Luna");
            assert_eq!(pending.face().label.as_ref(), face.label.as_ref());

            crate::omega_model_tier::clear_selection_for_test();
        });
    }

    /// The same guarantee in the direction the owner actually hit: the model
    /// the next turn starts on is Gemini, the standing static is a launch-fresh
    /// Luna, and the label must say Flash rather than Luna.
    #[gpui::test]
    fn the_label_follows_the_dispatch_when_the_standing_choice_is_launch_fresh(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let registry = install_registry_defaulting_to(
                "openagents",
                "gemini-3.6-flash",
                "Gemini 3.6 Flash",
                cx,
            );
            drop(registry);

            // A launch-fresh process: the static is Luna and nobody chose it.
            crate::omega_model_tier::clear_selection_for_test();
            assert_eq!(crate::omega_model_tier::selected(), ModelTier::Luna);

            let face = face_for_next_turn(None, crate::omega_model_tier::selected(), cx);
            assert_eq!(
                face.label.as_ref(),
                "Flash",
                "this is the owner's defect exactly: the composer said Luna \
                 while the send went to openagents/gemini-3.6-flash"
            );
            assert_ne!(face.label.as_ref(), "Luna");
            assert_eq!(face.model_name.as_ref(), "Gemini 3.6 Flash");

            crate::omega_model_tier::clear_selection_for_test();
        });
    }

    /// A process with no registry has no better answer than the standing
    /// choice, and must still draw a control rather than panicking.
    #[gpui::test]
    fn without_a_registry_the_standing_choice_is_the_last_resort(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(pending_routed_model(cx), None);
            let face = face_for_next_turn(None, ModelTier::Pro, cx);
            assert_eq!(face.label.as_ref(), "Pro");
        });
    }

    /// Register a single provider whose default model is the named pair, and
    /// make it the registry's default model.
    fn install_registry_defaulting_to(
        provider_id: &str,
        model_id: &str,
        model_name: &str,
        cx: &mut gpui::App,
    ) -> gpui::Entity<LanguageModelRegistry> {
        use language_model::fake_provider::{FakeLanguageModel, FakeLanguageModelProvider};
        use language_model::{
            ConfiguredModel, LanguageModel, LanguageModelProviderId, LanguageModelProviderName,
        };
        use std::sync::Arc;

        let model: Arc<dyn LanguageModel> = Arc::new(FakeLanguageModel::with_id_and_thinking(
            provider_id,
            model_id,
            model_name,
            false,
        ));
        let provider = Arc::new(
            FakeLanguageModelProvider::new(
                LanguageModelProviderId::from(provider_id.to_string()),
                LanguageModelProviderName::from(provider_id.to_string()),
            )
            .with_models(vec![model.clone()]),
        );

        language_model::init(cx);
        let registry = LanguageModelRegistry::global(cx);
        registry.update(cx, |registry, cx| {
            registry.register_provider(provider.clone(), cx);
            registry.set_default_model(
                Some(ConfiguredModel {
                    provider: provider.clone(),
                    model,
                }),
                cx,
            );
        });
        registry
    }

    /// An empty identifier is a bug, not a disclosure, and must not become a
    /// face that reads as a model called "".
    #[test]
    fn an_empty_identifier_is_not_a_routed_model() {
        let empty_model = native_disclosure("openagents", "");
        assert_eq!(RoutedModel::from_disclosure(&empty_model), None);
        let empty_provider = native_disclosure("", "kimi-k3");
        assert_eq!(RoutedModel::from_disclosure(&empty_provider), None);
    }
}
