//! The routed decision. `OMEGA-DELTA-0202`.
//!
//! One answer to "which model is serving this thread", and every label a person
//! reads is a function of it: the tier control's face, the executor disclosure
//! line, and the composer's status line.
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

    /// What the composer's status line reads.
    ///
    /// `OMEGA-DELTA-0202` holds this to **at most the model name**. The line
    /// used to carry the route receipt — dispatch reference, route summary,
    /// override mode, route-fallback state — none of which is a fact about the
    /// person's work, and the owner's standing no-exposition law applies to it.
    /// The route receipt is still recorded; it is simply not composer copy.
    #[must_use]
    pub fn status_line(&self) -> SharedString {
        self.face().model_name
    }
}

/// The face for a thread, falling back to the standing choice only when nothing
/// has been routed yet.
#[must_use]
pub fn face_for(routed: Option<&RoutedModel>, standing: ModelTier) -> RoutedFace {
    match routed {
        Some(routed) => routed.face(),
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

        let status = routed.status_line();
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
            assert_eq!(face.model_name, routed.status_line());
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
