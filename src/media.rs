//! Membaca lagu yang sedang diputar dari Windows (SMTC), di thread terpisah.
//! Tidak polling: Windows memanggil kita saat lagu, status, atau posisi berubah.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows::core::Result;
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Media::Control::*;
use windows::Storage::Streams::DataReader;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

#[derive(Clone)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Posisi (detik) pada saat `at`.
    pub position: f64,
    pub duration: f64,
    pub at: Instant,
    pub thumb: Option<Arc<Vec<u8>>>,
    /// Berubah setiap kali sampul berganti.
    pub thumb_id: u64,
}

impl MediaInfo {
    pub fn position_now(&self) -> f64 {
        let mut p = self.position;
        if self.playing {
            p += self.at.elapsed().as_secs_f64();
        }
        if self.duration > 0.0 {
            p = p.min(self.duration);
        }
        p.max(0.0)
    }
}

pub type Shared = Arc<Mutex<Option<MediaInfo>>>;

pub enum MediaCmd {
    PlayPause,
    Pause,
    Next,
    Prev,
    Seek(f64),
}

enum Msg {
    Cmd(MediaCmd),
    Session,
    Refresh,
}

#[derive(Clone)]
pub struct MediaHandle {
    tx: Sender<Msg>,
}

impl MediaHandle {
    pub fn send(&self, c: MediaCmd) {
        let _ = self.tx.send(Msg::Cmd(c));
    }
}

/// Mulai thread media. Setiap ada perubahan, `notify_msg` dikirim ke jendela `hwnd`.
pub fn spawn(hwnd: HWND, notify_msg: u32) -> (Shared, MediaHandle) {
    let shared: Shared = Arc::new(Mutex::new(None));
    let (tx, rx) = channel();
    let hwnd_raw = hwnd.0 as isize;
    {
        let shared = shared.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let notify = move || unsafe {
                let _ = PostMessageW(HWND(hwnd_raw as _), notify_msg, WPARAM(0), LPARAM(0));
            };
            if let Err(e) = run(&shared, tx, rx, &notify) {
                crate::crashlog::write(&format!("media: {e}"));
            }
        });
    }
    (shared, MediaHandle { tx })
}

type Session = GlobalSystemMediaTransportControlsSession;

struct Hooked {
    s: Session,
    tokens: [EventRegistrationToken; 3],
}

fn refresh_on<A: windows::core::RuntimeType + 'static>(
    tx: &Sender<Msg>,
) -> TypedEventHandler<Session, A> {
    let tx = tx.clone();
    TypedEventHandler::new(move |_, _| {
        let _ = tx.send(Msg::Refresh);
        Ok(())
    })
}

fn hook(s: Session, tx: &Sender<Msg>) -> Result<Hooked> {
    let t1 = s.MediaPropertiesChanged(&refresh_on(tx))?;
    let t2 = s.PlaybackInfoChanged(&refresh_on(tx))?;
    let t3 = s.TimelinePropertiesChanged(&refresh_on(tx))?;
    Ok(Hooked { s, tokens: [t1, t2, t3] })
}

fn unhook(h: &Hooked) {
    let _ = h.s.RemoveMediaPropertiesChanged(h.tokens[0]);
    let _ = h.s.RemovePlaybackInfoChanged(h.tokens[1]);
    let _ = h.s.RemoveTimelinePropertiesChanged(h.tokens[2]);
}

fn run(shared: &Shared, tx: Sender<Msg>, rx: Receiver<Msg>, notify: &dyn Fn()) -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
    {
        let tx = tx.clone();
        mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(Msg::Session);
            Ok(())
        }))?;
    }

    let mut cur: Option<Hooked> = None;
    let mut track = TrackState::default();
    let _ = tx.send(Msg::Session);

    while let Ok(first) = rx.recv() {
        // Event SMTC datang beruntun; kumpulkan jadi satu refresh.
        let mut msgs = vec![first];
        msgs.extend(rx.try_iter());
        let mut need_refresh = false;
        for m in msgs {
            match m {
                Msg::Session => {
                    if let Some(h) = cur.take() {
                        unhook(&h);
                    }
                    cur = mgr.GetCurrentSession().ok().and_then(|s| hook(s, &tx).ok());
                    need_refresh = true;
                }
                Msg::Refresh => need_refresh = true,
                Msg::Cmd(c) => {
                    if let Some(h) = &cur {
                        let _ = command(&h.s, c);
                    }
                }
            }
        }
        if need_refresh {
            let info = cur.as_ref().and_then(|h| read(&h.s, &mut track).ok());
            *shared.lock().unwrap() = info;
            notify();
        }
    }
    Ok(())
}

fn command(s: &Session, c: MediaCmd) -> Result<()> {
    match c {
        MediaCmd::PlayPause => s.TryTogglePlayPauseAsync()?.get()?,
        MediaCmd::Pause => s.TryPauseAsync()?.get()?,
        MediaCmd::Next => s.TrySkipNextAsync()?.get()?,
        MediaCmd::Prev => s.TrySkipPreviousAsync()?.get()?,
        MediaCmd::Seek(sec) => {
            let start = s.GetTimelineProperties()?.StartTime()?.Duration;
            s.TryChangePlaybackPositionAsync(start + (sec * 1e7) as i64)?.get()?
        }
    };
    Ok(())
}

#[derive(Default)]
struct TrackState {
    key: String,
    thumb: Option<Arc<Vec<u8>>>,
    thumb_id: u64,
}

fn read(s: &Session, st: &mut TrackState) -> Result<MediaInfo> {
    let props = s.TryGetMediaPropertiesAsync()?.get()?;
    let title = props.Title()?.to_string_lossy();
    let artist = props.Artist()?.to_string_lossy();
    let playing = s.GetPlaybackInfo()?.PlaybackStatus()?
        == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

    let tl = s.GetTimelineProperties()?;
    let start = tl.StartTime()?.Duration;
    let duration = (tl.EndTime()?.Duration - start) as f64 / 1e7;
    let mut position = (tl.Position()?.Duration - start) as f64 / 1e7;
    // Posisi dari pemutar bisa basi; majukan sejak terakhir diperbarui.
    let last = tl.LastUpdatedTime()?.UniversalTime;
    if playing && last > 0 {
        let now = unsafe { GetSystemTimeAsFileTime() };
        let now = ((now.dwHighDateTime as i64) << 32) | now.dwLowDateTime as i64;
        let ago = (now - last) as f64 / 1e7;
        if (0.0..3600.0).contains(&ago) {
            position += ago;
        }
    }

    let key = format!("{title}\u{1}{artist}");
    if key != st.key {
        st.key = key;
        st.thumb = None;
        st.thumb_id += 1;
    }
    // Browser sering mengirim logonya dulu lalu sampul asli belakangan,
    // jadi baca ulang setiap ada perubahan dan ganti kalau isinya berbeda.
    if let Some(b) = read_thumb(&props) {
        if st.thumb.as_deref() != Some(&b) {
            st.thumb = Some(Arc::new(b));
            st.thumb_id += 1;
        }
    }

    Ok(MediaInfo {
        title,
        artist,
        playing,
        position,
        duration: duration.max(0.0),
        at: Instant::now(),
        thumb: st.thumb.clone(),
        thumb_id: st.thumb_id,
    })
}

fn read_thumb(props: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Vec<u8>> {
    let r = props.Thumbnail().ok()?;
    let stream = r.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).ok()?;
    Some(buf)
}
