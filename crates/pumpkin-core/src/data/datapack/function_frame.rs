use crate::command::context::command_source::{CommandSource, ResultValueTaker, ReturnValue};
use std::sync::{Arc, Mutex};

struct Results {
    pending: usize,
    sum: Option<i32>,
}

// FunctionCommand.queueFunctionsNoReturn sums tags; queueFunctionsAsReturn uses the first return.
#[derive(Clone)]
pub(super) struct FunctionResult {
    results: Arc<Mutex<Results>>,
    callbacks: ResultValueTaker,
    single: bool,
    returning: Option<usize>,
    feedback: Option<(Arc<CommandSource>, String)>,
}

impl FunctionResult {
    pub(super) fn new(count: usize, callbacks: ResultValueTaker) -> Self {
        let returning = callbacks
            .0
            .iter()
            .find_map(|callback| callback.returned_function_frame());
        Self {
            results: Arc::new(Mutex::new(Results {
                pending: count,
                sum: None,
            })),
            callbacks,
            single: count == 1,
            returning,
            feedback: None,
        }
    }

    pub(super) fn record(&self, value: ReturnValue) {
        // FunctionCommand.decorateOutputIfNeeded reports each function's explicit result.
        if let Some((source, name)) = &self.feedback {
            let key = pumpkin_data::translation::java::COMMANDS_FUNCTION_RESULT;
            source.send_feedback(
                pumpkin_util::text::TextComponent::translate_cross(
                    key,
                    key,
                    [
                        pumpkin_util::text::TextComponent::text(name.clone()),
                        pumpkin_util::text::TextComponent::text(value.result_value().to_string()),
                    ],
                ),
                true,
            );
        }
        if self.returning.is_some() {
            let mut results = self
                .results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if results.sum.is_some() {
                return;
            }
            results.sum = Some(value.result_value());
            drop(results);
            self.callbacks.call(value);
        } else if self.single {
            self.callbacks.call(value);
        } else {
            let mut results = self
                .results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            results.sum = Some(results.sum.unwrap_or(0).wrapping_add(value.result_value()));
        }
    }

    pub(super) fn with_feedback(&self, source: Option<Arc<CommandSource>>, name: &str) -> Self {
        let mut result = self.clone();
        result.feedback = source.map(|source| (source, name.to_string()));
        result
    }

    #[must_use]
    pub(super) const fn returning_frame(&self) -> Option<usize> {
        self.returning
    }

    #[must_use]
    pub(super) fn should_discard(&self) -> bool {
        self.returning.is_some()
            && self
                .results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .sum
                .is_some()
    }

    pub(super) fn complete(&self) {
        let value = {
            let mut results = self
                .results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            results.pending -= 1;
            if results.pending != 0 {
                None
            } else if self.returning.is_some() && results.sum.is_none() {
                Some(ReturnValue::Failure)
            } else if self.returning.is_none() && !self.single {
                results.sum.map(ReturnValue::Success)
            } else {
                None
            }
        };
        if let Some(value) = value {
            self.callbacks.call(value);
        }
    }
}
