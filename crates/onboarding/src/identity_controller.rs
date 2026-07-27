use omega_identity::{CustodyResult, IdentityInspection, ReceiptRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IdentityOperation {
    Inspect,
    Create { receipt_ref: ReceiptRef },
    AdoptCustodied { receipt_ref: ReceiptRef },
    ResumeIncomplete,
    PrepareRecovery,
    AdoptRecovery { receipt_ref: ReceiptRef },
    ResolveConflict { receipt_ref: ReceiptRef },
    ExportRecovery,
    Reset { receipt_ref: ReceiptRef },
    ResumeReset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationToken {
    generation: u64,
    operation: IdentityOperation,
}

impl OperationToken {
    pub(crate) fn operation(&self) -> &IdentityOperation {
        &self.operation
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum IdentityUiError {
    InvalidSecret,
    InvalidPassword,
    UnsafeRecoveryFile,
    RecoveryFileExists,
    SecureStorageUnavailable,
    OperationFailed,
}

#[derive(Debug, Default)]
pub(crate) struct IdentityControllerState {
    generation: u64,
    durable: Option<IdentityInspection>,
    operation: Option<OperationToken>,
    error: Option<IdentityUiError>,
}

impl IdentityControllerState {
    pub(crate) fn durable(&self) -> Option<&IdentityInspection> {
        self.durable.as_ref()
    }

    pub(crate) fn operation(&self) -> Option<&IdentityOperation> {
        self.operation.as_ref().map(OperationToken::operation)
    }

    pub(crate) fn error(&self) -> Option<IdentityUiError> {
        self.error
    }

    pub(crate) fn begin(&mut self, operation: IdentityOperation) -> Option<OperationToken> {
        if self.operation.is_some() {
            return None;
        }
        Some(self.replace(operation))
    }

    pub(crate) fn replace(&mut self, operation: IdentityOperation) -> OperationToken {
        self.generation = self.generation.wrapping_add(1);
        let token = OperationToken {
            generation: self.generation,
            operation,
        };
        self.operation = Some(token.clone());
        self.error = None;
        token
    }

    pub(crate) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.operation = None;
        self.error = None;
    }

    pub(crate) fn cancel_if_current(&mut self, token: &OperationToken) -> bool {
        if self.operation.as_ref() != Some(token) || token.generation != self.generation {
            return false;
        }
        self.cancel();
        true
    }

    pub(crate) fn forget_durable(&mut self) {
        self.durable = None;
    }

    pub(crate) fn report_error(&mut self, error: IdentityUiError) {
        self.generation = self.generation.wrapping_add(1);
        self.operation = None;
        self.error = Some(error);
    }

    pub(crate) fn apply(
        &mut self,
        token: &OperationToken,
        result: Result<IdentityInspection, IdentityUiError>,
    ) -> bool {
        if self.operation.as_ref() != Some(token) || token.generation != self.generation {
            return false;
        }
        self.operation = None;
        match result {
            Ok(result) => {
                self.durable = Some(result);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
        true
    }

    pub(crate) fn apply_custody(
        &mut self,
        token: &OperationToken,
        result: Result<CustodyResult, IdentityUiError>,
    ) -> bool {
        if self.operation.as_ref() != Some(token) || token.generation != self.generation {
            return false;
        }
        self.operation = None;
        match result {
            Ok(custody) => {
                if let Some(inspection) = &mut self.durable {
                    inspection.custody = custody;
                    inspection.pending_transaction = None;
                    inspection.conflict = None;
                }
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
        true
    }

    pub(crate) fn accept(&mut self, token: &OperationToken) -> bool {
        if self.operation.as_ref() != Some(token) || token.generation != self.generation {
            return false;
        }
        self.operation = None;
        self.error = None;
        true
    }

    pub(crate) fn reject(&mut self, token: &OperationToken, error: IdentityUiError) -> bool {
        if self.operation.as_ref() != Some(token) || token.generation != self.generation {
            return false;
        }
        self.operation = None;
        self.error = Some(error);
        true
    }
}

#[cfg(test)]
mod tests {
    use omega_identity::{
        CustodyResult, CustodyState, RecoveryProtectionState, RecoveryProtectionStatus,
    };

    use super::*;

    fn result(state: CustodyState) -> IdentityInspection {
        IdentityInspection {
            custody: CustodyResult {
                state,
                identity: None,
                receipt_ref: None,
            },
            pending_transaction: None,
            conflict: None,
            recovery_protection: RecoveryProtectionStatus {
                state: RecoveryProtectionState::NotApplicable,
                record: None,
            },
        }
    }

    #[test]
    fn only_the_current_generation_and_phase_can_update_public_state() {
        let mut controller = IdentityControllerState::default();
        let first = controller.replace(IdentityOperation::Inspect);
        let second = controller.replace(IdentityOperation::ResumeIncomplete);

        assert!(!controller.apply(&first, Ok(result(CustodyState::Absent))));
        assert!(controller.durable().is_none());
        assert!(controller.apply(&second, Ok(result(CustodyState::Ready))));
        assert_eq!(
            controller
                .durable()
                .map(|inspection| inspection.custody.state),
            Some(CustodyState::Ready)
        );
    }

    #[test]
    fn errors_do_not_replace_the_last_durable_fact() {
        let mut controller = IdentityControllerState::default();
        let inspect = controller.replace(IdentityOperation::Inspect);
        assert!(controller.apply(&inspect, Ok(result(CustodyState::Lost))));

        let retry = controller.replace(IdentityOperation::Inspect);
        assert!(controller.apply(&retry, Err(IdentityUiError::SecureStorageUnavailable)));
        assert_eq!(
            controller
                .durable()
                .map(|inspection| inspection.custody.state),
            Some(CustodyState::Lost)
        );
        assert_eq!(
            controller.error(),
            Some(IdentityUiError::SecureStorageUnavailable)
        );
    }

    #[test]
    fn busy_operations_reject_double_submission_and_cancel_fences_late_results() {
        let mut controller = IdentityControllerState::default();
        let create_receipt = ReceiptRef::new("create-one").expect("valid receipt");
        let create = controller
            .begin(IdentityOperation::Create {
                receipt_ref: create_receipt,
            })
            .expect("start create");
        assert!(controller.begin(IdentityOperation::Inspect).is_none());

        controller.cancel();
        assert!(!controller.apply(&create, Ok(result(CustodyState::Ready))));
        assert!(controller.operation().is_none());
        assert!(controller.durable().is_none());
    }

    #[test]
    fn dialog_cancellation_only_cancels_its_own_operation() {
        let mut controller = IdentityControllerState::default();
        let dialog = controller.replace(IdentityOperation::PrepareRecovery);
        let newer = controller.replace(IdentityOperation::Inspect);

        assert!(!controller.cancel_if_current(&dialog));
        assert_eq!(controller.operation(), Some(&IdentityOperation::Inspect));
        assert!(controller.cancel_if_current(&newer));
        assert!(controller.operation().is_none());
    }

    #[test]
    fn durable_facts_can_be_forgotten_before_reinspection() {
        let mut controller = IdentityControllerState::default();
        let inspect = controller.replace(IdentityOperation::Inspect);
        assert!(controller.apply(&inspect, Ok(result(CustodyState::Ready))));

        controller.forget_durable();

        assert!(controller.durable().is_none());
    }
}
