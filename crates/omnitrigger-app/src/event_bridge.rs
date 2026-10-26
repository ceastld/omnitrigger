//! 将阻塞事件源转到有界 UI 队列，以合并唤醒替代短间隔轮询。

use std::{
    future::poll_fn,
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    task::{Poll, Waker},
    thread,
};

const FORWARD_CAPACITY: usize = 256;

#[derive(Default)]
struct WakeState {
    pending: bool,
    waker: Option<Waker>,
}

/// 单个 UI 消费者使用的合并唤醒信号，事件本身仍保存在有界队列中。
#[derive(Clone, Default)]
pub struct UiWake {
    state: Arc<Mutex<WakeState>>,
}

impl UiWake {
    pub fn new() -> Self {
        Self::default()
    }

    /// 保留一次待处理通知；在锁外唤醒，避免执行器回调重入同一把锁。
    pub fn notify(&self) {
        let waker = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.pending = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// 不并发等待同一信号；一次等待可合并多个生产者的通知。
    pub async fn wait(&self) {
        poll_fn(|cx| {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if std::mem::take(&mut state.pending) {
                state.waker = None;
                Poll::Ready(())
            } else {
                // 检查 pending 与注册 waker 共用锁，notify 不能穿过注册间隙。
                if !state
                    .waker
                    .as_ref()
                    .is_some_and(|existing| existing.will_wake(cx.waker()))
                {
                    state.waker = Some(cx.waker().clone());
                }
                Poll::Pending
            }
        })
        .await;
    }
}

/// 专用桥线程可以阻塞，输入 Hook 和 UI 线程不在转发队列上等待。
pub struct EventBridge<T> {
    receiver: Receiver<T>,
    wake: UiWake,
}

impl<T: Send + 'static> EventBridge<T> {
    pub fn new(source: Receiver<T>) -> Result<Self, String> {
        Self::with_wake(source, UiWake::new())
    }

    pub fn with_wake(source: Receiver<T>, wake: UiWake) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(FORWARD_CAPACITY);
        let producer_wake = wake.clone();
        let _worker = thread::Builder::new()
            .name("omnitrigger-event-bridge".into())
            .spawn(move || {
                while let Ok(event) = source.recv() {
                    // 只有桥线程承受背压；UI receiver 被释放时，send 会返回错误。
                    if sender.send(event).is_err() {
                        return;
                    }
                    producer_wake.notify();
                }
                // 消费者被唤醒后应立刻能看见 Disconnected，不能先唤醒再断开。
                drop(sender);
                producer_wake.notify();
            })
            .map_err(|error| format!("无法创建界面事件桥线程：{error}"))?;
        // 不在 UI Drop 中 join：源断开结束 recv；UI receiver 释放解除转发 send 背压。
        Ok(Self { receiver, wake })
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.receiver.try_recv()
    }

    pub fn wake(&self) -> UiWake {
        self.wake.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        pin::pin,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Barrier,
        },
        task::{Context, Wake},
        time::Duration,
    };

    #[derive(Default)]
    struct CountWake(AtomicUsize);

    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct ChannelWake(mpsc::Sender<()>);

    impl Wake for ChannelWake {
        fn wake(self: Arc<Self>) {
            let _ = self.0.send(());
        }
    }

    #[test]
    fn notification_before_wait_is_remembered_and_consumed_once() {
        let wake = UiWake::new();
        let counter = Arc::new(CountWake::default());
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        wake.notify();
        wake.notify();
        assert!(pin!(wake.wait()).as_mut().poll(&mut cx).is_ready());
        assert!(pin!(wake.wait()).as_mut().poll(&mut cx).is_pending());
        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn registered_waiter_is_woken_once_for_a_batch_of_notifications() {
        let wake = UiWake::new();
        let counter = Arc::new(CountWake::default());
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        let mut wait = pin!(wake.wait());
        assert!(wait.as_mut().poll(&mut cx).is_pending());
        for _ in 0..100 {
            wake.notify();
        }
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        assert!(wait.as_mut().poll(&mut cx).is_ready());
        assert!(pin!(wake.wait()).as_mut().poll(&mut cx).is_pending());
    }

    #[test]
    fn repolling_registers_the_current_executor_waker() {
        let wake = UiWake::new();
        let first = Arc::new(CountWake::default());
        let second = Arc::new(CountWake::default());
        let first_waker = Waker::from(first.clone());
        let second_waker = Waker::from(second.clone());
        let mut wait = pin!(wake.wait());
        assert!(wait
            .as_mut()
            .poll(&mut Context::from_waker(&first_waker))
            .is_pending());
        assert!(wait
            .as_mut()
            .poll(&mut Context::from_waker(&second_waker))
            .is_pending());
        wake.notify();
        assert_eq!(first.0.load(Ordering::SeqCst), 0);
        assert_eq!(second.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn executor_wake_callback_runs_after_unlocking_signal_state() {
        struct LockCheckingWake {
            signal: UiWake,
            unlocked: AtomicBool,
        }
        impl Wake for LockCheckingWake {
            fn wake(self: Arc<Self>) {
                self.unlocked
                    .store(self.signal.state.try_lock().is_ok(), Ordering::SeqCst);
            }
        }
        let signal = UiWake::new();
        let check = Arc::new(LockCheckingWake {
            signal: signal.clone(),
            unlocked: AtomicBool::new(false),
        });
        let waker = Waker::from(check.clone());
        let mut wait = pin!(signal.wait());
        assert!(wait
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending());
        signal.notify();
        assert!(check.unlocked.load(Ordering::SeqCst));
    }

    #[test]
    fn concurrent_notification_cannot_fall_between_check_and_registration() {
        for _ in 0..64 {
            let signal = UiWake::new();
            let barrier = Arc::new(Barrier::new(2));
            let producer_signal = signal.clone();
            let producer_barrier = barrier.clone();
            let producer = thread::spawn(move || {
                producer_barrier.wait();
                producer_signal.notify();
            });
            let (tx, rx) = mpsc::channel();
            let waker = Waker::from(Arc::new(ChannelWake(tx)));
            let mut cx = Context::from_waker(&waker);
            let mut wait = pin!(signal.wait());
            barrier.wait();
            let initial = wait.as_mut().poll(&mut cx);
            producer.join().unwrap();
            if initial.is_pending() {
                rx.recv_timeout(Duration::from_secs(2))
                    .expect("已注册的等待者必须收到并发通知");
                assert!(wait.as_mut().poll(&mut cx).is_ready());
            }
        }
    }

    #[test]
    fn bridge_preserves_event_order_until_source_disconnects() {
        let (tx, rx) = mpsc::channel();
        let bridge = EventBridge::new(rx).unwrap();
        assert_eq!(bridge.try_recv(), Err(TryRecvError::Empty));
        for value in 0..10 {
            tx.send(value).unwrap();
        }
        drop(tx);
        for value in 0..10 {
            assert_eq!(
                bridge.receiver.recv_timeout(Duration::from_secs(2)),
                Ok(value)
            );
        }
        assert_eq!(
            bridge.receiver.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn source_disconnect_wakes_waiter_after_forwarded_sender_is_dropped() {
        let (tx, rx) = mpsc::channel::<()>();
        let bridge = EventBridge::new(rx).unwrap();
        let signal = bridge.wake();
        let (wake_tx, wake_rx) = mpsc::channel();
        let waker = Waker::from(Arc::new(ChannelWake(wake_tx)));
        let mut cx = Context::from_waker(&waker);
        let mut wait = pin!(signal.wait());
        assert!(wait.as_mut().poll(&mut cx).is_pending());
        drop(tx);
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(bridge.try_recv(), Err(TryRecvError::Disconnected));
        assert!(wait.as_mut().poll(&mut cx).is_ready());
    }

    #[test]
    fn replacement_bridge_wakes_the_original_ui_signal() {
        let (first_tx, first_rx) = mpsc::channel::<u32>();
        let first = EventBridge::new(first_rx).unwrap();
        let signal = first.wake();
        let (wake_tx, wake_rx) = mpsc::channel();
        let waker = Waker::from(Arc::new(ChannelWake(wake_tx)));
        let mut cx = Context::from_waker(&waker);
        let mut first_wait = pin!(signal.wait());
        assert!(first_wait.as_mut().poll(&mut cx).is_pending());
        drop(first_tx);
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(first_wait.as_mut().poll(&mut cx).is_ready());
        assert_eq!(first.try_recv(), Err(TryRecvError::Disconnected));
        drop(first);

        let (next_tx, next_rx) = mpsc::channel();
        let next = EventBridge::with_wake(next_rx, signal.clone()).unwrap();
        let mut next_wait = pin!(signal.wait());
        assert!(next_wait.as_mut().poll(&mut cx).is_pending());
        next_tx.send(42).unwrap();
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(next_wait.as_mut().poll(&mut cx).is_ready());
        assert_eq!(next.try_recv(), Ok(42));
    }

    #[test]
    fn dropping_ui_receiver_releases_a_bridge_blocked_by_bounded_backpressure() {
        let (tx, rx) = mpsc::sync_channel(0);
        let bridge = EventBridge::new(rx).unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            for value in 0..FORWARD_CAPACITY + 2 {
                if tx.send(value).is_err() {
                    done_tx.send(value).unwrap();
                    return;
                }
                accepted_tx.send(value).unwrap();
            }
            done_tx.send(FORWARD_CAPACITY + 2).unwrap();
        });
        // 256 个已入队，另一个由桥线程持有并阻塞在 send；源的下一次发送等待 recv。
        for value in 0..=FORWARD_CAPACITY {
            assert_eq!(accepted_rx.recv_timeout(Duration::from_secs(2)), Ok(value));
        }
        assert_eq!(accepted_rx.try_recv(), Err(TryRecvError::Empty));
        drop(bridge);
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(2)),
            Ok(FORWARD_CAPACITY + 1)
        );
        producer.join().unwrap();
    }
}
