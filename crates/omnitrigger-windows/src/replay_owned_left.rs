//! ReplayRuntime的动作左键所有权扩展；实际交付仍由同一worker/标签/账本完成。
use super::*;
use crate::owned_left_input::{
    OwnedLeftGate, OwnedLeftInputError, OwnedLeftJobGuard, OwnedLeftRequest,
};
use omnitrigger_core::MouseButton;
#[derive(Default)]
pub(super) struct OwnedLeftState {
    press: Option<Press>,
    pending: Option<OwnedLeftRequest>,
    waiters: Vec<OwnedLeftRequest>,
    in_flight: bool,
}
struct Press {
    owner: u64,
    epoch: u64,
    gate: Arc<OwnedLeftGate>,
}
fn left() -> Button {
    Button::Mouse(MouseButton::Left)
}
fn reply(request: &OwnedLeftRequest, result: Result<(), OwnedLeftInputError>) {
    let _ = request.reply.try_send(result);
}
impl ReplayRuntime {
    fn reply_owned_waiters(&mut self, result: Result<(), OwnedLeftInputError>) {
        for request in self.owned_left.waiters.drain(..) {
            reply(&request, result);
        }
    }
    /// ACK发送端已经退出，不再有可等待的worker回执。
    /// 只终结请求与在途状态；delivered仍只记录确认过的交付。
    pub(super) fn finish_owned_left_worker_exit(&mut self, in_flight: Option<OwnedLeftRequest>) {
        let pending = self.owned_left.pending.take();
        let never_submitted = in_flight.is_none()
            && pending.as_ref().is_some_and(|request| request.down)
            && self
                .owned_left
                .press
                .as_ref()
                .is_none_or(|press| self.delivered.get(&left()) != Some(&press.epoch));
        let released_or_transferred = self
            .owned_left
            .press
            .as_ref()
            .is_none_or(|press| press.gate.revoked.load(Ordering::Acquire))
            || never_submitted;
        self.owned_left.in_flight = false;
        if released_or_transferred {
            // 未提交Down，或已观察到外部归属转交；不改写其它周期的交付账本。
            self.owned_left.press = None;
        } else if let Some(press) = &self.owned_left.press {
            // 保留原owner/epoch。已提交但ACK丢失的Down没有confirmed delivered，
            // 仍承担未知交付义务；已确认Down则保留原delivered，不能清债假成功。
            press.gate.cancelled.store(true, Ordering::Release);
            self.recovery.insert(left());
        }
        for request in pending.into_iter().chain(in_flight) {
            let result = if !request.down && released_or_transferred {
                Ok(())
            } else {
                Err(OwnedLeftInputError::new("input_owner_unavailable"))
            };
            reply(&request, result);
        }
        self.reply_owned_waiters(if released_or_transferred {
            Ok(())
        } else {
            Err(OwnedLeftInputError::new("input_owner_unavailable"))
        });
    }
    pub fn owned_left_active(&self) -> bool {
        self.owned_left.press.is_some()
    }
    pub fn owned_left_pending(&self) -> bool {
        self.owned_left.pending.is_some() || self.owned_left.in_flight
    }
    pub fn revoke_owned_left(&mut self) {
        if let Some(press) = &self.owned_left.press {
            press.gate.revoked.store(true, Ordering::Release);
        }
        if !self.owned_left.in_flight {
            if let Some(request) = self.owned_left.pending.take() {
                reply(
                    &request,
                    if request.down {
                        Err(OwnedLeftInputError::new("input_interrupted"))
                    } else {
                        Ok(())
                    },
                );
            }
            self.owned_left.press = None;
            self.reply_owned_waiters(Ok(()));
        }
    }
    pub fn cancel_owned_left(&mut self) {
        if let Some(press) = &self.owned_left.press {
            press.gate.cancelled.store(true, Ordering::Release);
            if !self.fatal_cleanup
                && self.owned_left.pending.is_none()
                && !self.owned_left.in_flight
            {
                self.owned_left.pending =
                    Some(OwnedLeftRequest::cleanup(press.owner, press.gate.clone()));
            }
        }
    }
    pub fn owned_left_stop_revoked(&self) -> bool {
        self.flight
            .as_ref()
            .and_then(|f| f.owned.as_ref())
            .is_some_and(|request| request.gate.revoked.load(Ordering::Acquire))
    }
    pub fn owned_left_request(&mut self, mut request: OwnedLeftRequest, preheld: bool) {
        if request.down {
            if preheld
                || self.fatal_cleanup
                || self.busy()
                || self.owned_left_active()
                || self.delivered.contains_key(&left())
            {
                reply(&request, Err(OwnedLeftInputError::new("input_busy")));
                return;
            }
            if request.gate.cancelled.load(Ordering::Acquire) || Instant::now() >= request.deadline
            {
                reply(&request, Err(OwnedLeftInputError::new("cancelled")));
                return;
            }
            self.next_epoch = self.next_epoch.wrapping_add(1);
            self.epochs.insert(left(), self.next_epoch);
            self.owned_left.press = Some(Press {
                owner: request.owner,
                epoch: self.next_epoch,
                gate: request.gate.clone(),
            });
        } else {
            let Some(press) = &self.owned_left.press else {
                reply(&request, Ok(()));
                return;
            };
            if press.owner != request.owner {
                reply(&request, Ok(()));
                return;
            }
            request.gate = press.gate.clone();
            if self.owned_left.pending.is_some() || self.owned_left.in_flight {
                request.gate.cancelled.store(true, Ordering::Release);
                // Endpoint最多8个保留槽；相同owner的清理等待实际ACK，不忙等耗尽预算。
                self.owned_left.waiters.push(request);
                return;
            }
        }
        self.owned_left.pending = Some(request);
    }
    pub(super) fn pump_owned_left(&mut self) -> bool {
        let Some(request) = self.owned_left.pending.take() else {
            return false;
        };
        let Some(press) = self.owned_left.press.as_ref() else {
            reply(&request, Ok(()));
            self.reply_owned_waiters(Ok(()));
            return false;
        };
        let revoked = press.gate.revoked.load(Ordering::Acquire);
        if revoked {
            reply(
                &request,
                if request.down {
                    Err(OwnedLeftInputError::new("input_interrupted"))
                } else {
                    Ok(())
                },
            );
            self.owned_left.press = None;
            self.reply_owned_waiters(Ok(()));
            return false;
        }
        if request.down
            && (request.gate.cancelled.load(Ordering::Acquire)
                || Instant::now() >= request.deadline)
        {
            reply(&request, Err(OwnedLeftInputError::new("cancelled")));
            self.owned_left.press = None;
            self.reply_owned_waiters(Ok(()));
            return false;
        }
        if !request.down && self.delivered.get(&left()) != Some(&press.epoch) {
            if self.recovery.contains(&left()) {
                // ACK退出留下的未知Down：没有confirmed交付记录，不代表没有债务。
                self.fatal_cleanup = true;
                let result = Err(OwnedLeftInputError::new("input_owner_unavailable"));
                reply(&request, result);
                self.reply_owned_waiters(result);
                return false;
            }
            reply(&request, Ok(()));
            self.owned_left.press = None;
            self.reply_owned_waiters(Ok(()));
            return false;
        }
        let down = request.down;
        let epoch = press.epoch;
        let kind = if down {
            InputKind::Down(left())
        } else {
            InputKind::Up(left())
        };
        let release = crate::owned_left_input::owned_left_button_native_input(false);
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let sequence = self.next_sequence;
        let tags = vec![Tag {
            kind,
            epoch,
            release: Some(release),
        }];
        let foreground = request.target.map_or(0, |target| target.window_handle);
        let job = Job {
            sequence,
            inputs: vec![crate::owned_left_input::owned_left_button_native_input(
                down,
            )],
            tags: tags.clone(),
            foreground,
            cleanup: !down,
            owned: Some(OwnedLeftJobGuard {
                gate: request.gate.clone(),
                down,
                target: request.target,
                deadline: request.deadline,
            }),
        };
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.jobs.try_send(Work::Submit(job)).is_ok())
        {
            self.owned_left.in_flight = true;
            self.flight = Some(Flight {
                sequence,
                tags,
                cleanup: !down,
                owned: Some(request),
            });
            true
        } else {
            reply(
                &request,
                Err(OwnedLeftInputError::new("input_owner_unavailable")),
            );
            if down {
                self.owned_left.press = None;
            } else {
                self.fatal_cleanup = true;
            }
            self.reply_owned_waiters(if down {
                Ok(())
            } else {
                Err(OwnedLeftInputError::new("input_owner_unavailable"))
            });
            false
        }
    }
    pub(super) fn finish_owned_left_ack(
        &mut self,
        request: OwnedLeftRequest,
        tags: &[Tag],
        sent: usize,
        skipped: bool,
    ) {
        self.owned_left.in_flight = false;
        let Some(press) = self.owned_left.press.as_ref() else {
            reply(&request, Err(OwnedLeftInputError::new("input_interrupted")));
            return;
        };
        let revoked = press.gate.revoked.load(Ordering::Acquire);
        if revoked {
            // 已通过的外部Down/Up优先；不能仅凭物理epoch变化丢弃尚未释放的API按下。
            self.owned_left.press = None;
            self.reply_owned_waiters(Ok(()));
            reply(
                &request,
                if request.down {
                    Err(OwnedLeftInputError::new("input_interrupted"))
                } else {
                    Ok(())
                },
            );
            return;
        }
        if sent == 1 && !skipped {
            if request.down {
                self.delivered.insert(left(), press.epoch);
                if let Some(release) = tags[0].release {
                    self.templates.insert(left(), release);
                }
                if request.gate.cancelled.load(Ordering::Acquire)
                    || Instant::now() >= request.deadline
                {
                    reply(&request, Err(OwnedLeftInputError::new("cancelled")));
                    let mut cleanup = request;
                    cleanup.down = false;
                    cleanup.target = None;
                    self.owned_left.pending = Some(cleanup);
                } else {
                    reply(&request, Ok(()));
                }
            } else {
                if self.delivered.get(&left()) == Some(&press.epoch) {
                    self.delivered.remove(&left());
                }
                self.owned_left.press = None;
                self.recovery
                    .retain(|button| self.delivered.contains_key(button));
                reply(&request, Ok(()));
                self.reply_owned_waiters(Ok(()));
            }
        } else {
            if request.down {
                self.owned_left.press = None;
            } else {
                self.fatal_cleanup = true;
            }
            // 未确认的Up保持同一delivered/owner；调用方可在总预算内继续收尾。
            reply(
                &request,
                Err(OwnedLeftInputError::new(if skipped {
                    "target_changed"
                } else {
                    "input_failed"
                })),
            );
            self.reply_owned_waiters(if request.down {
                Ok(())
            } else {
                Err(OwnedLeftInputError::new("input_failed"))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(
        owner: u64,
        down: bool,
    ) -> (
        OwnedLeftRequest,
        std::sync::mpsc::Receiver<Result<(), OwnedLeftInputError>>,
    ) {
        OwnedLeftRequest::fixture(owner, down, Arc::new(OwnedLeftGate::default()))
    }
    fn acknowledge(runtime: &mut ReplayRuntime, sent: usize, skipped: bool) {
        let request = runtime.owned_left.pending.take().unwrap();
        let epoch = runtime.owned_left.press.as_ref().unwrap().epoch;
        let kind = if request.down {
            InputKind::Down(left())
        } else {
            InputKind::Up(left())
        };
        runtime.owned_left.in_flight = true;
        runtime.finish_owned_left_ack(
            request,
            &[Tag {
                kind,
                epoch,
                release: None,
            }],
            sent,
            skipped,
        );
    }
    #[test]
    fn owned_left_ack_sequence_changes_the_existing_delivery_ledger_only_after_ack() {
        let mut runtime = ReplayRuntime::new();
        let (down, down_reply) = request(1, true);
        runtime.owned_left_request(down, false);
        assert!(!runtime.delivered.contains_key(&left()));
        acknowledge(&mut runtime, 1, false);
        down_reply.recv().unwrap().unwrap();
        assert!(runtime.delivered.contains_key(&left()));
        let (up, up_reply) = request(1, false);
        runtime.owned_left_request(up, false);
        acknowledge(&mut runtime, 1, false);
        up_reply.recv().unwrap().unwrap();
        assert!(!runtime.delivered.contains_key(&left()));
        assert!(!runtime.owned_left_active());
    }
    #[test]
    fn late_cancelled_down_ack_schedules_same_owner_neutral_cleanup() {
        let mut runtime = ReplayRuntime::new();
        let (down, reply) = request(2, true);
        let gate = down.gate.clone();
        runtime.owned_left_request(down, false);
        gate.cancelled.store(true, Ordering::Release);
        let (up, up_reply) = request(2, false);
        runtime.owned_left_request(up, false);
        assert!(matches!(
            up_reply.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        acknowledge(&mut runtime, 1, false);
        assert_eq!(reply.recv().unwrap().unwrap_err().code, "cancelled");
        assert!(!runtime.owned_left.pending.as_ref().unwrap().down);
        assert_eq!(runtime.owned_left.pending.as_ref().unwrap().owner, 2);
        acknowledge(&mut runtime, 1, false);
        up_reply.recv().unwrap().unwrap();
        assert!(!runtime.delivered.contains_key(&left()));
    }
    #[test]
    fn cancelled_pending_down_completes_coalesced_up_without_native_job() {
        let mut runtime = ReplayRuntime::new();
        let (down, down_reply) = request(6, true);
        runtime.owned_left_request(down, false);
        let (up, up_reply) = request(6, false);
        runtime.owned_left_request(up, false);
        assert!(!runtime.pump_owned_left());
        assert_eq!(down_reply.recv().unwrap().unwrap_err().code, "cancelled");
        up_reply.recv().unwrap().unwrap();
        assert!(!runtime.owned_left_active());
        assert!(runtime.owned_left.waiters.is_empty());
        assert!(!runtime.delivered.contains_key(&left()));
    }
    #[test]
    fn preheld_or_target_rejected_down_does_not_create_a_release_debt() {
        let mut runtime = ReplayRuntime::new();
        let (down, reply) = request(3, true);
        runtime.owned_left_request(down, true);
        assert_eq!(reply.recv().unwrap().unwrap_err().code, "input_busy");
        assert!(!runtime.owned_left_active());
        let (down, reply) = request(3, true);
        runtime.owned_left_request(down, false);
        acknowledge(&mut runtime, 0, true);
        assert_eq!(reply.recv().unwrap().unwrap_err().code, "target_changed");
        assert!(!runtime.delivered.contains_key(&left()));
        assert!(!runtime.owned_left_active());
    }
    #[test]
    fn passed_external_press_transfers_ownership_and_old_release_cannot_release_it() {
        let mut runtime = ReplayRuntime::new();
        let (down, reply) = request(4, true);
        runtime.owned_left_request(down, false);
        acknowledge(&mut runtime, 1, false);
        reply.recv().unwrap().unwrap();
        runtime.new_press(left());
        runtime.passed(InputKind::Down(left()), &BTreeMap::new());
        let epoch = runtime.delivered[&left()];
        let (up, reply) = request(4, false);
        runtime.owned_left_request(up, false);
        reply.recv().unwrap().unwrap();
        assert_eq!(runtime.delivered[&left()], epoch);
        assert!(runtime.owned_left.pending.is_none());
        runtime.passed(InputKind::Up(left()), &BTreeMap::new());
        assert!(!runtime.delivered.contains_key(&left()));
    }
    #[test]
    fn failed_owned_up_retains_ledger_until_a_confirmed_retry() {
        let mut runtime = ReplayRuntime::new();
        let (down, reply) = request(5, true);
        runtime.owned_left_request(down, false);
        acknowledge(&mut runtime, 1, false);
        reply.recv().unwrap().unwrap();
        let (up, reply) = request(5, false);
        runtime.owned_left_request(up, false);
        acknowledge(&mut runtime, 0, false);
        assert_eq!(reply.recv().unwrap().unwrap_err().code, "input_failed");
        assert!(runtime.delivered.contains_key(&left()));
        assert!(runtime.fatal_cleanup);
        runtime.cancel_owned_left();
        assert!(
            runtime.owned_left.pending.is_none(),
            "失败后不得由故障通知无限自动重试Up"
        );
        let (up, reply) = request(5, false);
        runtime.owned_left_request(up, false);
        assert!(runtime.fatal_cleanup);
        acknowledge(&mut runtime, 1, false);
        reply.recv().unwrap().unwrap();
        assert!(!runtime.delivered.contains_key(&left()));
        assert!(
            runtime.fatal_cleanup,
            "释放成功不能自动恢复因清理失败而暂停的监听"
        );
    }
}

#[cfg(test)]
#[path = "ownership_failure_sequences.rs"]
mod ownership_failure_sequences;

#[cfg(test)]
#[path = "owned_left_worker_exit_tests.rs"]
mod worker_exit_tests;
