//! 持续读取辅助进程候选的纯协议与超时状态机；没有 OS、进程或剪贴板调用。
//! 辅助进程只在 Capture 请求后读一次。Ready/空闲不采集；失败请求不自动重发。

use std::fmt;

pub const REQUEST_BYTES: usize = 21;
pub const REPLY_HEADER: usize = 25;
pub const MAX_UNITS: usize = 65_536;
pub const MAX_REPLY: usize = REPLY_HEADER + MAX_UNITS * 2;
const REQUEST_MAGIC: &[u8; 8] = b"OTHCMD01";
const REPLY_MAGIC: &[u8; 8] = b"OTHSTR01";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidFrame,
    InvalidUnicode,
    TooLarge,
    TrailingData,
    Truncated,
    UnexpectedReply,
    WrongRequest,
    InvalidState,
    InvalidBudget,
    CounterExhausted,
    TimeOverflow,
    StartupTimeout,
    ReadTimeout,
    StopTimeout,
    PeerRefused(CaptureFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Capture {
        request_id: u64,
        previous_sequence: u32,
    },
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SkipReason {
    Duplicate = 2,
    Transient = 3,
    Excluded = 4,
    Files = 5,
    Image = 6,
    Unsupported = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CaptureFailure {
    ScopeMismatch = 128,
    Busy = 129,
    Changed = 130,
    InvalidText = 131,
    TooLarge = 132,
    Backend = 133,
}

#[derive(PartialEq, Eq)]
pub enum Reply {
    Ready,
    StartFailed {
        reason: CaptureFailure,
    },
    Text {
        request_id: u64,
        sequence: u32,
        text: String,
    },
    Skipped {
        request_id: u64,
        sequence: u32,
        reason: SkipReason,
    },
    Failed {
        request_id: u64,
        reason: CaptureFailure,
    },
    Stopped,
}

impl fmt::Debug for Reply {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => output.write_str("Ready"),
            Self::Stopped => output.write_str("Stopped"),
            Self::StartFailed { reason } => {
                output.debug_tuple("StartFailed").field(reason).finish()
            }
            Self::Text {
                request_id,
                sequence,
                text,
            } => output
                .debug_struct("Text")
                .field("request_id", request_id)
                .field("sequence", sequence)
                .field("utf16_units", &text.encode_utf16().count())
                .finish(),
            Self::Skipped {
                request_id,
                sequence,
                reason,
            } => output
                .debug_struct("Skipped")
                .field("request_id", request_id)
                .field("sequence", sequence)
                .field("reason", reason)
                .finish(),
            Self::Failed { request_id, reason } => output
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("reason", reason)
                .finish(),
        }
    }
}

pub fn encode_request(request: Request) -> Result<[u8; REQUEST_BYTES], ProtocolError> {
    let mut bytes = [0; REQUEST_BYTES];
    bytes[..8].copy_from_slice(REQUEST_MAGIC);
    if let Request::Capture {
        request_id,
        previous_sequence,
    } = request
    {
        if request_id == 0 {
            return Err(ProtocolError::InvalidFrame);
        }
        bytes[8] = 1;
        bytes[9..17].copy_from_slice(&request_id.to_le_bytes());
        bytes[17..21].copy_from_slice(&previous_sequence.to_le_bytes());
    } else {
        bytes[8] = 2;
    }
    Ok(bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<Request, ProtocolError> {
    if bytes.len() != REQUEST_BYTES || &bytes[..8] != REQUEST_MAGIC {
        return Err(ProtocolError::InvalidFrame);
    }
    let request_id = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
    let previous_sequence = u32::from_le_bytes(bytes[17..21].try_into().unwrap());
    match bytes[8] {
        1 if request_id != 0 => Ok(Request::Capture {
            request_id,
            previous_sequence,
        }),
        2 if request_id == 0 && previous_sequence == 0 => Ok(Request::Stop),
        _ => Err(ProtocolError::InvalidFrame),
    }
}

pub fn encode_reply(reply: &Reply) -> Result<Vec<u8>, ProtocolError> {
    let (tag, request_id, sequence, text) = match reply {
        Reply::Ready => (0, 0, 0, None),
        Reply::StartFailed { reason } => (*reason as u8, 0, 0, None),
        Reply::Text {
            request_id,
            sequence,
            text,
        } => (1, *request_id, *sequence, Some(text)),
        Reply::Skipped {
            request_id,
            sequence,
            reason,
        } => (*reason as u8, *request_id, *sequence, None),
        Reply::Failed { request_id, reason } => (*reason as u8, *request_id, 0, None),
        Reply::Stopped => (8, 0, 0, None),
    };
    if !matches!(
        reply,
        Reply::Ready | Reply::Stopped | Reply::StartFailed { .. }
    ) && request_id == 0
    {
        return Err(ProtocolError::InvalidFrame);
    }
    let units: Vec<u16> = match text {
        Some(text) => {
            if text.contains('\0') {
                return Err(ProtocolError::InvalidUnicode);
            }
            if text.encode_utf16().count() > MAX_UNITS {
                return Err(ProtocolError::TooLarge);
            }
            text.encode_utf16().collect()
        }
        None => vec![],
    };
    let mut bytes = Vec::with_capacity(REPLY_HEADER + units.len() * 2);
    bytes.extend_from_slice(REPLY_MAGIC);
    bytes.push(tag);
    bytes.extend_from_slice(&request_id.to_le_bytes());
    bytes.extend_from_slice(&sequence.to_le_bytes());
    bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(bytes)
}

fn decode_complete(bytes: &[u8]) -> Result<Reply, ProtocolError> {
    let request_id = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
    let sequence = u32::from_le_bytes(bytes[17..21].try_into().unwrap());
    let count = u32::from_le_bytes(bytes[21..25].try_into().unwrap()) as usize;
    let tag = bytes[8];
    if matches!(tag, 0 | 8) {
        if request_id != 0 || sequence != 0 || count != 0 {
            return Err(ProtocolError::InvalidFrame);
        }
        return Ok(if tag == 0 {
            Reply::Ready
        } else {
            Reply::Stopped
        });
    }
    if request_id == 0 && tag < 128 {
        return Err(ProtocolError::InvalidFrame);
    }
    if tag == 1 {
        let units: Vec<u16> = bytes[REPLY_HEADER..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16(&units).map_err(|_| ProtocolError::InvalidUnicode)?;
        if text.contains('\0') {
            return Err(ProtocolError::InvalidUnicode);
        }
        return Ok(Reply::Text {
            request_id,
            sequence,
            text,
        });
    }
    if count != 0 {
        return Err(ProtocolError::InvalidFrame);
    }
    if tag >= 128 {
        if sequence != 0 {
            return Err(ProtocolError::InvalidFrame);
        }
        let reason = match tag {
            128 => CaptureFailure::ScopeMismatch,
            129 => CaptureFailure::Busy,
            130 => CaptureFailure::Changed,
            131 => CaptureFailure::InvalidText,
            132 => CaptureFailure::TooLarge,
            133 => CaptureFailure::Backend,
            _ => return Err(ProtocolError::InvalidFrame),
        };
        return Ok(if request_id == 0 {
            Reply::StartFailed { reason }
        } else {
            Reply::Failed { request_id, reason }
        });
    }
    let reason = match tag {
        2 => SkipReason::Duplicate,
        3 => SkipReason::Transient,
        4 => SkipReason::Excluded,
        5 => SkipReason::Files,
        6 => SkipReason::Image,
        7 => SkipReason::Unsupported,
        _ => return Err(ProtocolError::InvalidFrame),
    };
    Ok(Reply::Skipped {
        request_id,
        sequence,
        reason,
    })
}

/// 同时只接受一个预期帧。多帧拼接、额外尾部和故障后的继续喂入均明确拒绝。
#[derive(Default)]
pub struct Decoder {
    bytes: Vec<u8>,
    failed: bool,
}

impl Decoder {
    fn push_inner(&mut self, input: &[u8]) -> Result<Option<Reply>, ProtocolError> {
        if self.failed {
            return Err(ProtocolError::InvalidState);
        }
        if input.len() > MAX_REPLY - self.bytes.len() {
            return Err(ProtocolError::TooLarge);
        }
        self.bytes.extend_from_slice(input);
        let prefix = self.bytes.len().min(8);
        if self.bytes[..prefix] != REPLY_MAGIC[..prefix] {
            return Err(ProtocolError::InvalidFrame);
        }
        if self.bytes.len() < REPLY_HEADER {
            return Ok(None);
        }
        let count = u32::from_le_bytes(self.bytes[21..25].try_into().unwrap()) as usize;
        if count > MAX_UNITS {
            return Err(ProtocolError::TooLarge);
        }
        let expected = REPLY_HEADER + count * 2;
        if self.bytes.len() > expected {
            return Err(ProtocolError::TrailingData);
        }
        if self.bytes.len() < expected {
            return Ok(None);
        }
        let reply = decode_complete(&self.bytes)?;
        self.bytes.clear();
        Ok(Some(reply))
    }

    pub fn push(&mut self, input: &[u8]) -> Result<Option<Reply>, ProtocolError> {
        let result = self.push_inner(input);
        if result.is_err() {
            self.failed = true;
            self.bytes.clear();
        }
        result
    }

    pub fn end_of_stream(&mut self) -> Result<(), ProtocolError> {
        if self.failed {
            return Err(ProtocolError::InvalidState);
        }
        if self.bytes.is_empty() {
            return Ok(());
        }
        self.failed = true;
        self.bytes.clear();
        Err(ProtocolError::Truncated)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    pub startup_ms: u64,
    pub read_ms: u64,
    pub stop_grace_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerState {
    Stopped,
    AwaitReady { deadline: u64 },
    Ready,
    AwaitReply { request_id: u64, deadline: u64 },
    AwaitStop { deadline: u64 },
    Dead,
}

/// Dead 或协议/超时错误要求适配层立即终止并回收子进程、丢弃解码器和所有旧字节。
/// 不发 Spawn/重试命令；只有新的显式调用可再次 begin_start。
pub struct WorkerMachine {
    state: WorkerState,
    budgets: Budgets,
    next_request: u64,
    now: u64,
}

impl WorkerMachine {
    pub fn new(budgets: Budgets) -> Result<Self, ProtocolError> {
        if !(1..=60_000).contains(&budgets.startup_ms)
            || !(1..=60_000).contains(&budgets.read_ms)
            || budgets.stop_grace_ms > 5000
        {
            return Err(ProtocolError::InvalidBudget);
        }
        Ok(Self {
            state: WorkerState::Stopped,
            budgets,
            next_request: 1,
            now: 0,
        })
    }
    pub fn state(&self) -> WorkerState {
        self.state
    }
    fn time(&mut self, now: u64) -> u64 {
        self.now = self.now.max(now);
        self.now
    }
    fn deadline(&mut self, now: u64, budget: u64) -> Result<u64, ProtocolError> {
        self.time(now)
            .checked_add(budget)
            .ok_or(ProtocolError::TimeOverflow)
    }
    pub fn begin_start(&mut self, now: u64) -> Result<(), ProtocolError> {
        if !matches!(self.state, WorkerState::Stopped | WorkerState::Dead) {
            return Err(ProtocolError::InvalidState);
        }
        let deadline = self.deadline(now, self.budgets.startup_ms)?;
        self.state = WorkerState::AwaitReady { deadline };
        Ok(())
    }
    pub fn capture(&mut self, previous_sequence: u32, now: u64) -> Result<Request, ProtocolError> {
        if self.state != WorkerState::Ready {
            return Err(ProtocolError::InvalidState);
        }
        let Some(next) = self.next_request.checked_add(1) else {
            self.state = WorkerState::Dead;
            return Err(ProtocolError::CounterExhausted);
        };
        let deadline = match self.deadline(now, self.budgets.read_ms) {
            Ok(value) => value,
            Err(error) => {
                self.state = WorkerState::Dead;
                return Err(error);
            }
        };
        let request_id = self.next_request;
        self.next_request = next;
        self.state = WorkerState::AwaitReply {
            request_id,
            deadline,
        };
        Ok(Request::Capture {
            request_id,
            previous_sequence,
        })
    }
    pub fn begin_stop(&mut self, now: u64) -> Result<Option<Request>, ProtocolError> {
        if matches!(
            self.state,
            WorkerState::Stopped | WorkerState::Dead | WorkerState::AwaitStop { .. }
        ) {
            return Ok(None);
        }
        let deadline = match self.deadline(now, self.budgets.stop_grace_ms) {
            Ok(value) => value,
            Err(error) => {
                self.state = WorkerState::Dead;
                return Err(error);
            }
        };
        self.state = WorkerState::AwaitStop { deadline };
        Ok(Some(Request::Stop))
    }
    pub fn poll(&mut self, now: u64) -> Result<(), ProtocolError> {
        let now = self.time(now);
        let error = match self.state {
            WorkerState::AwaitReady { deadline } if now >= deadline => {
                Some(ProtocolError::StartupTimeout)
            }
            WorkerState::AwaitReply { deadline, .. } if now >= deadline => {
                Some(ProtocolError::ReadTimeout)
            }
            WorkerState::AwaitStop { deadline } if now >= deadline => {
                Some(ProtocolError::StopTimeout)
            }
            _ => None,
        };
        if let Some(error) = error {
            self.state = WorkerState::Dead;
            return Err(error);
        }
        Ok(())
    }
    pub fn accept(&mut self, reply: Reply, now: u64) -> Result<Option<Reply>, ProtocolError> {
        self.poll(now)?;
        match (self.state, reply) {
            (WorkerState::AwaitReady { .. }, Reply::Ready) => {
                self.state = WorkerState::Ready;
                Ok(None)
            }
            (WorkerState::AwaitReady { .. }, Reply::StartFailed { reason }) => {
                self.state = WorkerState::Dead;
                Err(ProtocolError::PeerRefused(reason))
            }
            (WorkerState::AwaitStop { .. }, Reply::Stopped) => {
                self.state = WorkerState::Stopped;
                Ok(None)
            }
            (WorkerState::AwaitReply { request_id, .. }, reply) => {
                let received = match &reply {
                    Reply::Text { request_id, .. }
                    | Reply::Skipped { request_id, .. }
                    | Reply::Failed { request_id, .. } => *request_id,
                    _ => {
                        self.state = WorkerState::Dead;
                        return Err(ProtocolError::UnexpectedReply);
                    }
                };
                if received != request_id {
                    self.state = WorkerState::Dead;
                    return Err(ProtocolError::WrongRequest);
                }
                self.state = WorkerState::Ready;
                Ok(Some(reply))
            }
            _ => {
                self.state = WorkerState::Dead;
                Err(ProtocolError::UnexpectedReply)
            }
        }
    }
    pub fn peer_exited(&mut self) {
        self.state = WorkerState::Dead;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn new_machine() -> WorkerMachine {
        WorkerMachine::new(Budgets {
            startup_ms: 5000,
            read_ms: 1000,
            stop_grace_ms: 50,
        })
        .unwrap()
    }
    fn text(id: u64, value: &str) -> Reply {
        Reply::Text {
            request_id: id,
            sequence: u32::MAX,
            text: value.into(),
        }
    }
    fn ready(machine: &mut WorkerMachine) {
        machine.begin_start(0).unwrap();
        assert_eq!(machine.accept(Reply::Ready, 1), Ok(None));
    }

    #[test]
    fn request_frames_are_fixed_size_canonical_and_never_contain_text() {
        for request in [
            Request::Stop,
            Request::Capture {
                request_id: u64::MAX,
                previous_sequence: u32::MAX,
            },
        ] {
            let bytes = encode_request(request).unwrap();
            assert_eq!(decode_request(&bytes), Ok(request));
        }
        assert_eq!(
            encode_request(Request::Capture {
                request_id: 0,
                previous_sequence: 1
            }),
            Err(ProtocolError::InvalidFrame)
        );
        let mut bad = encode_request(Request::Stop).unwrap();
        bad[17] = 1;
        assert_eq!(decode_request(&bad), Err(ProtocolError::InvalidFrame));
    }
    #[test]
    fn replies_can_be_split_at_every_byte_and_preserve_unicode() {
        let expected = text(9007199254740993, " \r\n文😀 ");
        let bytes = encode_reply(&expected).unwrap();
        for split in 1..bytes.len() {
            let mut decoder = Decoder::default();
            assert_eq!(decoder.push(&bytes[..split]), Ok(None));
            assert_eq!(
                decoder.push(&bytes[split..]),
                Ok(Some(text(9007199254740993, " \r\n文😀 ")))
            );
        }
    }
    #[test]
    fn decoder_is_bounded_and_errors_poison_the_connection() {
        let bytes = encode_reply(&Reply::Ready).unwrap();
        let mut decoder = Decoder::default();
        assert_eq!(
            decoder.push(&[bytes.clone(), bytes].concat()),
            Err(ProtocolError::TrailingData)
        );
        assert_eq!(decoder.push(&[]), Err(ProtocolError::InvalidState));
        assert_eq!(
            Decoder::default().push(&vec![0; MAX_REPLY + 1]),
            Err(ProtocolError::TooLarge)
        );
    }
    #[test]
    fn truncated_and_malformed_unicode_frames_do_not_return_partial_text() {
        let mut bytes = encode_reply(&text(1, "x")).unwrap();
        let mut decoder = Decoder::default();
        assert_eq!(decoder.push(&bytes[..REPLY_HEADER]), Ok(None));
        assert_eq!(decoder.end_of_stream(), Err(ProtocolError::Truncated));
        bytes[REPLY_HEADER..].copy_from_slice(&0xd800_u16.to_le_bytes());
        assert_eq!(
            Decoder::default().push(&bytes),
            Err(ProtocolError::InvalidUnicode)
        );
    }
    #[test]
    fn ready_never_requests_capture_and_idle_has_no_polling_timeout() {
        let mut machine = new_machine();
        ready(&mut machine);
        assert_eq!(machine.state(), WorkerState::Ready);
        assert_eq!(machine.poll(1_000_000), Ok(()));
        assert_eq!(machine.state(), WorkerState::Ready);
    }
    #[test]
    fn warm_requests_reuse_ready_state_with_strictly_new_ids() {
        let mut machine = new_machine();
        ready(&mut machine);
        for id in 1..=3 {
            assert_eq!(
                machine.capture(17, id * 10),
                Ok(Request::Capture {
                    request_id: id,
                    previous_sequence: 17
                })
            );
            assert_eq!(
                machine.accept(text(id, "synthetic"), id * 10 + 1),
                Ok(Some(text(id, "synthetic")))
            );
            assert_eq!(machine.state(), WorkerState::Ready);
        }
    }
    #[test]
    fn read_timeout_kills_connection_without_retry_and_late_reply_is_rejected() {
        let mut machine = new_machine();
        ready(&mut machine);
        machine.capture(1, 10).unwrap();
        assert_eq!(machine.poll(1010), Err(ProtocolError::ReadTimeout));
        assert_eq!(machine.state(), WorkerState::Dead);
        assert_eq!(
            machine.accept(text(1, "synthetic late"), 1011),
            Err(ProtocolError::UnexpectedReply)
        );
        assert_eq!(machine.capture(1, 1012), Err(ProtocolError::InvalidState));
    }
    #[test]
    fn cold_start_budget_and_request_budget_are_separate_not_performance_claims() {
        let mut machine = new_machine();
        machine.begin_start(0).unwrap();
        assert_eq!(machine.poll(1000), Ok(()));
        assert_eq!(machine.accept(Reply::Ready, 4000), Ok(None));
        machine.capture(0, 4000).unwrap();
        assert_eq!(machine.poll(4999), Ok(()));
        assert_eq!(machine.poll(5000), Err(ProtocolError::ReadTimeout));
    }
    #[test]
    fn startup_timeout_and_unsolicited_reply_fail_closed() {
        let mut machine = new_machine();
        machine.begin_start(0).unwrap();
        assert_eq!(machine.poll(5000), Err(ProtocolError::StartupTimeout));
        let mut machine = new_machine();
        ready(&mut machine);
        assert_eq!(
            machine.accept(text(1, "synthetic unsolicited"), 2),
            Err(ProtocolError::UnexpectedReply)
        );
    }
    #[test]
    fn stale_request_ids_after_restart_are_never_accepted() {
        let mut machine = new_machine();
        ready(&mut machine);
        machine.capture(0, 2).unwrap();
        machine.peer_exited();
        machine.begin_start(3).unwrap();
        machine.accept(Reply::Ready, 4).unwrap();
        assert_eq!(
            machine.capture(0, 5),
            Ok(Request::Capture {
                request_id: 2,
                previous_sequence: 0
            })
        );
        assert_eq!(
            machine.accept(text(1, "synthetic stale"), 6),
            Err(ProtocolError::WrongRequest)
        );
    }
    #[test]
    fn stop_deadline_rejects_late_text_and_does_not_wait_for_rendering() {
        let mut machine = new_machine();
        ready(&mut machine);
        machine.capture(0, 2).unwrap();
        assert_eq!(machine.begin_stop(3), Ok(Some(Request::Stop)));
        assert_eq!(
            machine.accept(text(1, "synthetic late"), 4),
            Err(ProtocolError::UnexpectedReply)
        );
        let mut machine = new_machine();
        ready(&mut machine);
        machine.begin_stop(3).unwrap();
        assert_eq!(machine.poll(53), Err(ProtocolError::StopTimeout));
    }
    #[test]
    fn fixed_capture_error_can_leave_process_ready_without_replaying_operation() {
        let mut machine = new_machine();
        ready(&mut machine);
        machine.capture(0, 2).unwrap();
        let reply = Reply::Failed {
            request_id: 1,
            reason: CaptureFailure::Busy,
        };
        assert_eq!(
            machine.accept(reply, 3),
            Ok(Some(Reply::Failed {
                request_id: 1,
                reason: CaptureFailure::Busy
            }))
        );
        assert_eq!(machine.state(), WorkerState::Ready);
        assert_eq!(machine.poll(10_000), Ok(()));
    }
    #[test]
    fn deadline_time_and_request_overflow_never_wrap() {
        let mut machine = new_machine();
        assert_eq!(
            machine.begin_start(u64::MAX),
            Err(ProtocolError::TimeOverflow)
        );
        let mut machine = new_machine();
        ready(&mut machine);
        machine.next_request = u64::MAX;
        assert_eq!(machine.capture(0, 2), Err(ProtocolError::CounterExhausted));
        assert_eq!(machine.state(), WorkerState::Dead);
    }
    #[test]
    fn reply_debug_does_not_echo_body() {
        assert!(
            !format!("{:?}", text(1, "synthetic-private-正文")).contains("synthetic-private-正文")
        );
    }

    #[test]
    fn failed_scope_handshake_is_fixed_and_can_never_reach_ready_or_capture() {
        let reply = Reply::StartFailed {
            reason: CaptureFailure::ScopeMismatch,
        };
        let bytes = encode_reply(&reply).unwrap();
        assert_eq!(bytes.len(), REPLY_HEADER);
        assert_eq!(Decoder::default().push(&bytes), Ok(Some(reply)));
        let mut machine = new_machine();
        machine.begin_start(0).unwrap();
        assert_eq!(
            machine.accept(
                Reply::StartFailed {
                    reason: CaptureFailure::ScopeMismatch
                },
                1
            ),
            Err(ProtocolError::PeerRefused(CaptureFailure::ScopeMismatch))
        );
        assert_eq!(machine.state(), WorkerState::Dead);
        assert_eq!(machine.capture(0, 2), Err(ProtocolError::InvalidState));
    }

    #[test]
    fn repeated_stop_does_not_extend_the_original_grace_deadline() {
        let mut machine = new_machine();
        ready(&mut machine);
        machine.begin_stop(10).unwrap();
        assert_eq!(machine.begin_stop(59), Ok(None));
        assert_eq!(machine.poll(60), Err(ProtocolError::StopTimeout));
    }

    #[test]
    fn reentrant_capture_cannot_replace_the_inflight_id_or_create_second_work() {
        let mut machine = new_machine();
        ready(&mut machine);
        let first = machine.capture(1, 10).unwrap();
        assert_eq!(machine.capture(2, 11), Err(ProtocolError::InvalidState));
        assert_eq!(
            first,
            Request::Capture {
                request_id: 1,
                previous_sequence: 1
            }
        );
        assert_eq!(
            machine.accept(text(1, "synthetic"), 12),
            Ok(Some(text(1, "synthetic")))
        );
        assert_eq!(
            machine.capture(3, 13),
            Ok(Request::Capture {
                request_id: 2,
                previous_sequence: 3
            })
        );
    }

    #[test]
    fn oversized_declared_body_is_rejected_before_allocating_or_receiving_body() {
        let mut header = encode_reply(&Reply::Ready).unwrap();
        header[21..25].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Decoder::default().push(&header),
            Err(ProtocolError::TooLarge)
        );
    }

    #[test]
    fn reserved_control_fields_and_payloads_on_errors_fail_closed() {
        for offset in [9, 17, 21] {
            let mut bytes = encode_reply(&Reply::Ready).unwrap();
            bytes[offset] = 1;
            if offset == 21 {
                bytes.extend([b'x', 0]);
            }
            assert_eq!(
                Decoder::default().push(&bytes),
                Err(ProtocolError::InvalidFrame)
            );
        }
        let mut error = encode_reply(&Reply::Failed {
            request_id: 1,
            reason: CaptureFailure::Busy,
        })
        .unwrap();
        error[17] = 1;
        assert_eq!(
            Decoder::default().push(&error),
            Err(ProtocolError::InvalidFrame)
        );
        let mut error = encode_reply(&Reply::Failed {
            request_id: 1,
            reason: CaptureFailure::Busy,
        })
        .unwrap();
        error[21] = 1;
        error.extend([b'x', 0]);
        assert_eq!(
            Decoder::default().push(&error),
            Err(ProtocolError::InvalidFrame)
        );
    }

    #[test]
    fn deadline_equality_rejects_even_valid_text_and_time_backwards_does_not_extend_it() {
        let mut machine = new_machine();
        ready(&mut machine);
        machine.capture(0, 100).unwrap();
        assert_eq!(machine.poll(50), Ok(()));
        assert_eq!(
            machine.accept(text(1, "synthetic late"), 1100),
            Err(ProtocolError::ReadTimeout)
        );
    }

    #[test]
    fn text_and_skip_replies_preserve_zero_sequence_empty_text_and_max_u64_id() {
        for reply in [
            Reply::Text {
                request_id: u64::MAX,
                sequence: 0,
                text: String::new(),
            },
            Reply::Skipped {
                request_id: u64::MAX,
                sequence: 0,
                reason: SkipReason::Transient,
            },
        ] {
            let bytes = encode_reply(&reply).unwrap();
            assert_eq!(Decoder::default().push(&bytes), Ok(Some(reply)));
        }
    }
}
