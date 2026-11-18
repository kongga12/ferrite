//! ConPTY의 마지막 출력을 받는 PTY 감싸개.
//!
//! ConPTY(conhost)는 프로세스와 따로 화면을 내보내서, 곧바로 끝나는 명령의 마지막 출력은 프로세스가
//! 끝난 뒤에 파이프로 온다(재어 보니 4~16ms 뒤). alacritty_terminal의 이벤트 루프는 프로세스가 끝난 것을
//! 알면 그때까지 온 것만 읽고 끝나 이 출력을 버린다. 그래서 출력이 잠잠해진 뒤에 종료를 알린다.

use std::io::{self, Read};
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite};
use polling::os::iocp::{CompletionPacket, PollerIocpExt};
use polling::{Event, PollMode, Poller};

/// 프로세스가 끝난 뒤 이만큼 출력이 없으면 다 온 것으로 본다. 끝을 알리는 신호가 없어 시간으로 판단한다.
const QUIET: Duration = Duration::from_millis(250);
/// 끝난 명령이 남긴 프로세스가 계속 출력해도 이 시간이 지나면 종료를 알린다.
const LIMIT: Duration = Duration::from_secs(2);

pub(crate) struct Pty {
    inner: tty::Pty,
    poller: Option<Arc<Poller>>,
    /// 프로세스가 끝났으면 그 종료 상태와 알게 된 때.
    exit: Option<(Option<ExitStatus>, Instant)>,
    last_output: Instant,
}

impl Pty {
    pub(crate) fn new(inner: tty::Pty) -> Self {
        Self {
            inner,
            poller: None,
            exit: None,
            last_output: Instant::now(),
        }
    }
}

impl Read for Pty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.reader().read(buf)?;
        if read > 0 {
            self.last_output = Instant::now();
        }
        Ok(read)
    }
}

impl EventedReadWrite for Pty {
    type Reader = Self;
    type Writer = <tty::Pty as EventedReadWrite>::Writer;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        self.poller = Some(poll.clone());
        // SAFETY: 호출자가 지켜야 할 조건을 그대로 넘긴다(`inner`는 `self`와 수명이 같다).
        unsafe { self.inner.register(poll, interest, mode) }
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        self.inner.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        self
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}

impl EventedPty for Pty {
    /// 프로세스가 끝났어도 출력이 잠잠해질 때까지는 `None`을 돌려주고, 그때 다시 불리도록 이벤트 루프를
    /// 깨운다. 그동안 이벤트 루프는 늦게 온 출력을 평소처럼 읽는다.
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        if self.exit.is_none() {
            let ChildEvent::Exited(status) = self.inner.next_child_event()?;
            self.exit = Some((status, Instant::now()));
        }
        let (status, exited_at) = self.exit?;
        let settled = self.last_output.max(exited_at) + QUIET;
        let wait = settled
            .min(exited_at + LIMIT)
            .saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return Some(ChildEvent::Exited(status));
        }
        let poller = self.poller.clone()?;
        let wake = std::thread::Builder::new()
            .name("conpty-exit-wait".to_owned())
            .spawn(move || {
                std::thread::sleep(wait);
                let event = Event::readable(tty::PTY_CHILD_EVENT_TOKEN);
                let _ = poller.post(CompletionPacket::new(event));
            });
        // 깨울 스레드를 못 띄우면 기다리지 않는다(종료를 영영 알리지 못하는 것보다 낫다).
        wake.is_err().then_some(ChildEvent::Exited(status))
    }
}

impl OnResize for Pty {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size);
    }
}
