use super::DatapackManager;
use super::function_frame::FunctionResult;
use crate::command::context::command_source::{CommandSource, ResultValueTaker, ReturnValue};
use crate::server::Server;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[path = "function_execution_context.rs"]
mod execution_context;

// ExecutionContext.MAX_QUEUE_DEPTH in vanilla 26.3, line 19.
const MAX_QUEUE_DEPTH: usize = 10_000_000;

#[derive(Debug, PartialEq, Eq)]
pub enum FunctionRunError {
    Unknown(String),
    EmptyTag(String),
    ChainLimitExceeded,
}

impl fmt::Display for FunctionRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(reason) => f.write_str(reason),
            Self::EmptyTag(name) => write!(f, "No functions in tag: {name}"),
            Self::ChainLimitExceeded => f.write_str("Command execution stopped due to limit"),
        }
    }
}

struct Frame {
    id: usize,
    lines: Arc<[String]>,
    index: usize,
    started: bool,
    source: Arc<CommandSource>,
    result: FunctionResult,
    returned: bool,
}

enum Work {
    Command(Arc<CommandSource>, String),
    Completed(FunctionResult),
    Discarded,
}

type FunctionBody = (String, Arc<[String]>);

struct FunctionSource {
    source: Arc<CommandSource>,
    result: FunctionResult,
    feedback_source: Option<Arc<CommandSource>>,
}

struct CallBatch {
    lines: Arc<[FunctionBody]>,
    sources: Vec<Option<FunctionSource>>,
    source_index: usize,
    function_index: usize,
}

enum Entry {
    Batch(CallBatch),
    Frame,
}

impl CallBatch {
    fn remaining_calls(&self) -> usize {
        (self.sources.len() - self.source_index)
            .saturating_mul(self.lines.len())
            .saturating_sub(self.function_index)
    }

    // ContinuationTask.schedule materialises only the next CallFunction in source order.
    fn next_frame(&mut self) -> Option<Frame> {
        while let Some(source) = self.sources.get_mut(self.source_index) {
            let source_ref = source.as_ref()?;
            if source_ref.result.should_discard() {
                source.take();
                self.source_index += 1;
                self.function_index = 0;
                continue;
            }
            if let Some((name, lines)) = self.lines.get(self.function_index) {
                let frame = Frame {
                    id: 0,
                    lines: lines.clone(),
                    index: 0,
                    started: false,
                    source: source_ref.source.clone(),
                    result: source_ref
                        .result
                        .with_feedback(source_ref.feedback_source.clone(), name),
                    returned: false,
                };
                self.function_index += 1;
                if self.function_index == self.lines.len() {
                    source.take();
                    self.source_index += 1;
                    self.function_index = 0;
                }
                return Some(frame);
            }
            source.take();
            self.source_index += 1;
            self.function_index = 0;
        }
        None
    }
}

struct FunctionQueue {
    frames: Vec<Frame>,
    entries: Vec<Entry>,
    pending: Vec<Entry>,
    pending_returns: VecDeque<(FunctionResult, ReturnValue)>,
    delivering_returns: bool,
    remaining: usize,
    next_id: usize,
    queued_calls: usize,
    max_queue_depth: usize,
    queue_overflow: bool,
    fork_limit: usize,
}

thread_local! {
    static ACTIVE_QUEUE: RefCell<Option<FunctionQueue>> = const { RefCell::new(None) };
}

struct QueueGuard;

impl Drop for QueueGuard {
    fn drop(&mut self) {
        ACTIVE_QUEUE.with(|queue| *queue.borrow_mut() = None);
    }
}

/// True while function calls are being queued inside the current execution context.
#[must_use]
pub fn is_nested_function_call() -> bool {
    ACTIVE_QUEUE.with(|queue| queue.borrow().is_some())
}

/// Charges an executor or redirect to the active vanilla execution quota.
#[must_use]
pub fn consume_command_cost() -> bool {
    ACTIVE_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.as_mut().is_none_or(|queue| {
            if queue.queue_overflow || queue.remaining == 0 {
                return false;
            }
            queue.remaining -= 1;
            true
        })
    })
}

/// Identifies the current function frame for a deferred return callback.
#[must_use]
pub fn current_function_frame() -> Option<usize> {
    ACTIVE_QUEUE.with(|queue| queue.borrow().as_ref()?.frames.last().map(|frame| frame.id))
}

/// Discards a function's tail and forwards its explicit return once, as `ReturnCommand` does.
pub fn return_from_function(id: usize, value: ReturnValue) {
    let deliver = ACTIVE_QUEUE.with(|active| {
        let mut active = active.borrow_mut();
        let Some(queue) = active.as_mut() else {
            return false;
        };
        let Some(frame) = queue.frame_mut(id) else {
            return false;
        };
        frame.index = frame.lines.len();
        if frame.returned {
            return false;
        }
        frame.returned = true;
        let result = frame.result.clone();
        queue.pending_returns.push_back((result, value));
        if queue.delivering_returns {
            return false;
        }
        queue.delivering_returns = true;
        true
    });
    if !deliver {
        return;
    }
    // CallFunction's frame returns may target a parent; drain that chain without Rust recursion.
    while let Some((result, value)) = ACTIVE_QUEUE.with(|active| {
        let mut active = active.borrow_mut();
        let queue = active.as_mut()?;
        let result = queue.pending_returns.pop_front();
        if result.is_none() {
            queue.delivering_returns = false;
        }
        result
    }) {
        result.record(value);
    }
}

/// Discards the caller's remaining entries before executing `return run`.
pub fn discard_function_tail(id: usize) {
    ACTIVE_QUEUE.with(|queue| {
        if let Some(frame) = queue
            .borrow_mut()
            .as_mut()
            .and_then(|queue| queue.frame_mut(id))
        {
            frame.index = frame.lines.len();
        }
    });
}

fn command_limit(source: &CommandSource) -> usize {
    source.world.as_ref().map_or_else(
        || {
            pumpkin_data::game_rules::GameRuleRegistry::default()
                .max_command_sequence_length
                .max(1) as usize
        },
        |world| {
            world
                .level_info
                .load()
                .game_rules
                .max_command_sequence_length
                .max(1) as usize
        },
    )
}

/// Returns the active context's captured fork limit, or the source world's rule outside a context.
#[must_use]
pub fn max_command_forks(source: &CommandSource) -> usize {
    ACTIVE_QUEUE.with(|active| {
        active.borrow().as_ref().map_or_else(
            || {
                source.world.as_ref().map_or_else(
                    || {
                        pumpkin_data::game_rules::GameRuleRegistry::default().max_command_forks
                            as usize
                    },
                    |world| world.level_info.load().game_rules.max_command_forks.max(0) as usize,
                )
            },
            |queue| queue.fork_limit,
        )
    })
}

fn enqueue(batch: CallBatch) -> Result<(), FunctionRunError> {
    ACTIVE_QUEUE.with(|queue| {
        if let Some(queue) = queue.borrow_mut().as_mut() {
            queue.enqueue(batch)?;
        }
        Ok(())
    })
}

// Commands.executeCommandInContext: reentrant dispatch joins the active context.
fn run_in_context<T>(
    limit: usize,
    fork_limit: usize,
    max_queue_depth: usize,
    configure: impl FnOnce() -> T,
    mut dispatch: impl FnMut(&Arc<CommandSource>, &str),
) -> (T, usize) {
    if is_nested_function_call() {
        return (configure(), 0);
    }
    ACTIVE_QUEUE.with(|active| {
        *active.borrow_mut() = Some(FunctionQueue::new(limit, fork_limit, max_queue_depth));
    });
    let _guard = QueueGuard;
    let result = configure();
    let mut executed = 0;
    loop {
        let next =
            ACTIVE_QUEUE.with(|active| active.borrow_mut().as_mut().and_then(FunctionQueue::next));
        match next {
            Some(Work::Command(source, line)) => {
                dispatch(&source, &line);
                executed += 1;
            }
            Some(Work::Completed(result)) => result.complete(),
            Some(Work::Discarded) => {}
            None => break,
        }
    }
    ACTIVE_QUEUE.with(|active| {
        if let Some(queue) = active.borrow().as_ref() {
            // ExecutionContext.runCommandQueue logs overflow separately from exhausted quota.
            if queue.queue_overflow {
                tracing::error!("Command execution stopped due to command queue overflow (max {max_queue_depth})");
            } else if queue.remaining == 0 {
                tracing::info!("Command execution stopped due to limit (executed {limit} commands)");
            }
        }
    });
    (result, executed)
}

/// Installs the top-level command queue, dispatches, then drains deferred function calls.
/// Reentrant dispatch shares the current thread's queue and its remaining quota.
pub fn execute_command_in_context<T>(source: &CommandSource, dispatch: impl FnOnce() -> T) -> T {
    let Some(server) = &source.server else {
        return dispatch();
    };
    run_in_context(
        command_limit(source),
        max_command_forks(source),
        MAX_QUEUE_DEPTH,
        dispatch,
        |source, line| {
            server
                .command_dispatcher
                .load()
                .handle_command_with_source(source, line);
        },
    )
    .0
}

// FunctionCommand.FunctionCustomExecutor.runGuarded reports before queueing.
fn signal_scheduled(source: &CommandSource, lines: &[FunctionBody]) {
    let key = if lines.len() == 1 {
        pumpkin_data::translation::java::COMMANDS_FUNCTION_SCHEDULED_SINGLE
    } else {
        pumpkin_data::translation::java::COMMANDS_FUNCTION_SCHEDULED_MULTIPLE
    };
    source.send_feedback(
        pumpkin_util::text::TextComponent::translate_cross(
            key,
            key,
            [pumpkin_util::text::TextComponent::text(
                lines
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )],
        ),
        true,
    );
}

impl DatapackManager {
    fn function_tag_names(&self, tag: &str) -> Option<Vec<String>> {
        let mut names = self
            .function_tags
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(tag)?
            .clone();
        // TagLoader.build uses an insertion-ordered set, so duplicate entries run once.
        let mut seen = std::collections::HashSet::new();
        names.retain(|name| seen.insert(name.clone()));
        Some(names)
    }

    fn function_batch(
        &self,
        sources: &[Arc<CommandSource>],
        name: &str,
        feedback: bool,
    ) -> Result<CallBatch, FunctionRunError> {
        let (names, is_tag) = if let Some(tag) = name.strip_prefix('#') {
            let names = self.function_tag_names(tag).ok_or_else(|| {
                FunctionRunError::Unknown(format!("Unknown function tag: #{tag}"))
            })?;
            (names, true)
        } else {
            (vec![name.to_string()], false)
        };
        let functions = self
            .functions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut lines = Vec::new();
        for name in names {
            if let Some(body) = functions.get(&name) {
                lines.push((name, body.clone()));
            } else if !is_tag {
                return Err(FunctionRunError::Unknown(format!(
                    "Unknown function: {name}"
                )));
            }
        }
        drop(functions);
        if is_tag && lines.is_empty() {
            return Err(FunctionRunError::EmptyTag(name.to_string()));
        }
        let sources = sources
            .iter()
            .map(|source| {
                let result = FunctionResult::new(lines.len(), source.command_result_taker.clone());
                let feedback_source = (feedback && !source.silent).then(|| source.clone());
                if feedback_source.is_some() {
                    signal_scheduled(source, &lines);
                }
                // FunctionCommand.modifySenderForExecution / clearCallbacks reuse an unchanged sender.
                let source = if source.silent && source.command_result_taker.0.is_empty() {
                    source.clone()
                } else {
                    Arc::new(
                        source
                            .as_ref()
                            .clone()
                            .with_silent()
                            .with_command_result_taker(ResultValueTaker::new()),
                    )
                };
                Some(FunctionSource {
                    source,
                    result,
                    feedback_source,
                })
            })
            .collect();
        Ok(CallBatch {
            lines: lines.into(),
            sources,
            source_index: 0,
            function_index: 0,
        })
    }

    /// Queues nested calls and drains the outer call without recursive Rust dispatch.
    pub fn execute_function(
        &self,
        server: &Arc<Server>,
        source: &CommandSource,
        name: &str,
    ) -> Result<usize, FunctionRunError> {
        let batch = self.function_batch(&[Arc::new(source.clone())], name, false)?;
        Self::run_batch(server, source, batch)
    }

    /// Queues one lazy continuation for all sources of a function call site.
    pub fn execute_function_sources(
        &self,
        server: &Arc<Server>,
        sources: &[Arc<CommandSource>],
        name: &str,
    ) -> Result<usize, FunctionRunError> {
        let Some(source) = sources.first() else {
            return Ok(0);
        };
        let batch = self.function_batch(sources, name, true)?;
        Self::run_batch(server, source, batch)
    }

    fn run_batch(
        server: &Arc<Server>,
        source: &CommandSource,
        batch: CallBatch,
    ) -> Result<usize, FunctionRunError> {
        let (result, executed) = run_in_context(
            command_limit(source),
            max_command_forks(source),
            MAX_QUEUE_DEPTH,
            || enqueue(batch),
            |source, line| {
                server
                    .command_dispatcher
                    .load()
                    .handle_command_with_source(source, line);
            },
        );
        result.map(|()| executed)
    }

    #[cfg(test)]
    fn run_function(
        &self,
        source: &CommandSource,
        name: &str,
        limit: usize,
        mut dispatch: impl FnMut(&CommandSource, &str),
    ) -> Result<usize, FunctionRunError> {
        let batch = self.function_batch(&[Arc::new(source.clone())], name, false)?;
        let (result, executed) = run_in_context(
            limit,
            max_command_forks(source),
            MAX_QUEUE_DEPTH,
            || enqueue(batch),
            |source, line| dispatch(source, line),
        );
        result.map(|()| executed)
    }

    /// Marks the latest library for load-tag execution on the next game tick.
    pub fn mark_load_pending(&self) {
        self.load_pending.store(true, Ordering::Release);
    }

    /// Runs ServerFunctionManager.tick's pending load tag before the tick tag.
    pub fn tick_functions(&self, server: &Arc<Server>, source: &CommandSource) {
        if !server.tick_rate_manager.runs_normally() {
            return;
        }
        self.visit_tick_tags(|tag| {
            self.execute_tag_functions(server, source, tag);
        });
    }

    // ServerFunctionManager.executeTagFunctions gives each load/tick function its own context.
    fn execute_tag_functions(&self, server: &Arc<Server>, source: &CommandSource, tag: &str) {
        let names = self
            .function_tag_names(tag.trim_start_matches('#'))
            .unwrap_or_default();
        for name in names {
            let _ = self.execute_function(server, source, &name);
        }
    }

    fn visit_tick_tags(&self, mut execute: impl FnMut(&str)) {
        if self.load_pending.swap(false, Ordering::AcqRel) {
            execute("#minecraft:load");
        }
        execute("#minecraft:tick");
    }
}

#[cfg(test)]
#[path = "function_runner_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "function_continuation_tests.rs"]
mod continuation_tests;

#[cfg(test)]
#[path = "function_queue_tests.rs"]
mod queue_tests;
