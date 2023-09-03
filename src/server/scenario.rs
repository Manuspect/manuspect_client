use crate::client::{
    // SERVER_CLIPBOARD_ENABLED, SERVER_FILE_TRANSFER_ENABLED, SERVER_KEYBOARD_ENABLED, KEY_MAP,
    // check_if_retry, handle_hash, handle_login_from_ui, handle_test_delay, input_os_password,
    load_config,
    send_mouse,
    start_video_audio_threads,
    Client,
    CodecFormat,
    FileManager,
    Key,
    LoginConfigHandler,
    MediaData,
    MediaSender,
    QualityStatus,
    KEY_MAP,
    MILLI1,
    SEC30,
};
use crate::common::GrabState;
use crate::ui::remote::TauriHandler;
use crate::ui_session_interface::{InvokeUiSession, IS_IN};
use std::any::Any;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::SystemTime;

use crate::client::Data;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::common::{check_clipboard, update_clipboard, ClipboardContext, CLIPBOARD_INTERVAL};
use crate::ui_cm_interface::{ConnectionManager, InvokeUiCM};
use crate::{common, keyboard};
use async_trait::async_trait;
#[cfg(windows)]
use clipboard::{cliprdr::CliprdrClientContext, ContextSend};
use hbb_common::message_proto::permission_info::Permission;
use hbb_common::protos::message::Message;
use hbb_common::rendezvous_proto::ConnType;
#[cfg(windows)]
use hbb_common::tokio::sync::Mutex as TokioMutex;
use hbb_common::{allow_err, get_version_number, message_proto::*, sleep};
use hbb_common::{
    config::Config,
    fs, log,
    message_proto::EncodedVideoFrame,
    tokio::{
        self,
        sync::mpsc,
        time::{self, Duration, Instant, Interval},
    },
    Stream,
};
use rdev::{Event, EventType};
use std::collections::{HashMap, HashSet};

use super::{input_service::*, *};

// pub async fn start_listener(
//     tx_to_local_stream: mpsc::UnboundedSender<Message>,
//     mut rx_from_local_stream: mpsc::UnboundedReceiver<Message>,
// ) -> ResultType<()> {
//     Ok(())
// }

pub type Sender = mpsc::UnboundedSender<(Instant, Arc<Message>)>;

pub type HandlerPtr = Arc<RwLock<TauriBot>>;
pub type HandlerPtrWeak = Weak<RwLock<TauriBot>>;

pub fn new() -> HandlerPtr {
    let handler = TauriBot::new();
    Arc::new(RwLock::new(handler))
}

#[derive(Clone, Default)]
pub struct TauriBot(Bot<TauriHandler>);

impl Deref for TauriBot {
    type Target = Bot<TauriHandler>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TauriBot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TauriBot {
    pub fn new() -> Self {
        let bot: Bot<TauriHandler> = Bot {
            ..Default::default()
        };

        // let conn_type = ConnType::DEFAULT_CONN;
        // session
        //     .lc
        //     .write()
        //     .unwrap()
        //     .initialize(id, conn_type, None, false);

        Self(bot)
    }

    pub fn set_id(&mut self, id: String) {
        self.0.id = id;
    }

    pub fn reconnect(
        &self,
        mut rx_to_local_stream: mpsc::UnboundedReceiver<Message>,
        tx_from_local_stream: mpsc::UnboundedSender<Message>,
    ) {
        self.0
            .reconnect(false, rx_to_local_stream, tx_from_local_stream)
    }

    pub fn inner(&self) -> Bot<TauriHandler> {
        self.0.clone()
    }
}

/// Interface for client to send data and commands.
#[async_trait]
pub trait CustomInterface: Send + Clone + 'static + Sized {
    /// Send message data to remote peer.
    fn send(&self, data: Data);
    fn msgbox(&self, msgtype: &str, title: &str, text: &str, link: &str);
    fn handle_login_error(&mut self, err: &str) -> bool;
    fn handle_peer_info(&mut self, pi: PeerInfo);
    fn on_error(&self, err: &str) {
        self.msgbox("error", "Error", err, "");
    }
    async fn handle_login_from_ui(&mut self, password: String, remember: bool, peer: &mut Stream);
    async fn handle_test_delay(
        &mut self,
        t: TestDelay,
        tx_from_local_stream: &mpsc::UnboundedSender<Message>,
    );

    fn get_login_config_handler(&self) -> Arc<RwLock<LoginConfigHandler>>;
    fn set_force_relay(&self, direct: bool, received: bool) {
        self.get_login_config_handler()
            .write()
            .unwrap()
            .set_force_relay(direct, received);
    }
    fn is_force_relay(&self) -> bool {
        self.get_login_config_handler().read().unwrap().force_relay
    }
}

#[derive(Clone, Default)]
pub struct Bot<T: InvokeUiSession> {
    pub id: String,
    pub thread: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
    pub sender: Arc<RwLock<Option<mpsc::UnboundedSender<Data>>>>,
    pub ui_handler: T,
}

#[async_trait]
impl<T: InvokeUiSession> CustomInterface for Bot<T> {
    fn send(&self, data: Data) {
        if let Some(sender) = self.sender.read().unwrap().as_ref() {
            sender.send(data).ok();
        }
    }

    fn msgbox(&self, msgtype: &str, title: &str, text: &str, link: &str) {
        // let direct = self.lc.read().unwrap().direct.unwrap_or_default();
        // let received = self.lc.read().unwrap().received;
        // let retry_for_relay = direct && !received;
        // let retry = check_if_retry(msgtype, title, text, retry_for_relay);
        // self.ui_handler.msgbox(msgtype, title, text, link, retry);
    }

    fn handle_login_error(&mut self, err: &str) -> bool {
        todo!()
    }

    fn handle_peer_info(&mut self, mut pi: PeerInfo) {
        log::debug!("handle_peer_info :{:?}", pi);
        // pi.username = self.lc.read().unwrap().get_username(&pi);
        // if pi.current_display as usize >= pi.displays.len() {
        //     pi.current_display = 0;
        // }
        if get_version_number(&pi.version) < get_version_number("1.1.10") {
            self.ui_handler.set_permission("restart", false);
        }
        if self.is_file_transfer() {
            if pi.username.is_empty() {
                self.msgbox(
                    "error",
                    "Error",
                    "No active console user logged on, please connect and logon first.",
                    "",
                );
                return;
            }
        } else if !self.is_port_forward() {
            if pi.displays.is_empty() {
                // self.lc.write().unwrap().handle_peer_info(&pi);
                self.update_privacy_mode();
                self.msgbox("error", "Remote Error", "No Display", "");
                return;
            }
            // let p = self.lc.read().unwrap().should_auto_login();
            // if !p.is_empty() {
            //     input_os_password(p, true, self.clone());
            // }
            let current = &pi.displays[pi.current_display as usize];
            self.ui_handler.set_display(
                current.x,
                current.y,
                current.width,
                current.height,
                current.cursor_embedded,
            );
        }
        self.ui_handler.update_privacy_mode();
        // Save recent peers, then push event to flutter. So flutter can refresh peer page.
        self.on_connected();
        #[cfg(windows)]
        {
            let mut path = std::env::temp_dir();
            path.push(&self.id);
            let path = path.with_extension(crate::get_app_name().to_lowercase());
            std::fs::File::create(&path).ok();
            if let Some(path) = path.to_str() {
                crate::platform::windows::add_recent_document(&path);
            }
        }
    }

    async fn handle_login_from_ui(&mut self, password: String, remember: bool, peer: &mut Stream) {
        todo!()
    }

    async fn handle_test_delay(
        &mut self,
        t: TestDelay,
        tx_from_local_stream: &mpsc::UnboundedSender<Message>,
    ) {
        if !t.from_client {
            // self.update_quality_status(QualityStatus {
            //     delay: Some(t.last_delay as _),
            //     target_bitrate: Some(t.target_bitrate as _),
            //     ..Default::default()
            // });

            let mut msg_out = Message::new();
            msg_out.set_test_delay(t);
            allow_err!(tx_from_local_stream.send(msg_out));
        }
    }
    fn get_login_config_handler(&self) -> Arc<RwLock<LoginConfigHandler>> {
        todo!()
    }

    fn on_error(&self, err: &str) {
        self.msgbox("error", "Error", err, "");
    }

    fn set_force_relay(&self, direct: bool, received: bool) {
        self.get_login_config_handler()
            .write()
            .unwrap()
            .set_force_relay(direct, received);
    }

    fn is_force_relay(&self) -> bool {
        self.get_login_config_handler().read().unwrap().force_relay
    }
}

impl<T: InvokeUiSession> Bot<T> {
    pub fn is_file_transfer(&self) -> bool {
        false
    }

    pub fn is_rdp(&self) -> bool {
        false
    }

    pub fn is_port_forward(&self) -> bool {
        false
    }

    pub fn is_restarting_remote_device(&self) -> bool {
        // self.lc.read().unwrap().restarting_remote_device
        false
    }

    fn cancel_msgbox(&self, tag: &str) {
        // self.call_tauri("cancel_msgbox", tag);
    }

    pub fn set_display(&self, x: i32, y: i32, w: i32, h: i32, _cursor_embeded: bool) {
        // self.call_tauri("setDisplay", (x, y, w, h, _cursor_embeded));
        // TODO: start, stop streaming
        log::info!("[video] reinitialized");
        //     // https://sciter.com/forums/topic/color_spaceiyuv-crash
        //     // Nothing spectacular in decoder – done on CPU side.
        //     // So if you can do BGRA translation on your side – the better.
        //     // BGRA is used as internal image format so it will not require additional transformations.
        //     VIDEO.lock().unwrap().as_mut().map(|v| {
        //         v.stop_streaming().ok();
        //         let ok = v.start_streaming((w, h), COLOR_SPACE::Rgb32, None);
        //         log::info!("[video] reinitialized: {:?}", ok);
        //     });
    }

    pub fn switch_display(&self, display: &SwitchDisplay) {
        // self.call_tauri("switchDisplay", display.display);
    }

    pub fn set_permission(&self, name: &str, value: bool) {
        // self.call_tauri("setPermission", (name, value));
    }

    fn update_privacy_mode(&self) {
        // self.call_tauri("updatePrivacyMode", ());
    }

    pub fn get_keyboard_mode(&self) -> String {
        // self.lc.read().unwrap().keyboard_mode.clone()
        return "legacy".to_string();
    }

    #[inline]
    pub fn peer_platform(&self) -> String {
        // self.lc.read().unwrap().info.platform.clone()
        return "Windows".to_string();
    }

    pub fn enter(&self) {
        // #[cfg(target_os = "windows")]
        // {
        //     match &self.lc.read().unwrap().keyboard_mode as _ {
        //         "legacy" => rdev::set_get_key_unicode(true),
        //         "translate" => rdev::set_get_key_unicode(true),
        //         _ => {}
        //     }
        // }
        rdev::set_get_key_unicode(true);
        IS_IN.store(true, Ordering::SeqCst);
        keyboard::client::change_grab_status(GrabState::Run, true);
    }

    pub fn leave(&self) {
        #[cfg(target_os = "windows")]
        {
            rdev::set_get_key_unicode(false);
        }
        IS_IN.store(false, Ordering::SeqCst);
        keyboard::client::change_grab_status(GrabState::Wait, true);
    }

    // flutter only TODO new input
    pub fn input_key(
        &self,
        name: &str,
        down: bool,
        press: bool,
        alt: bool,
        ctrl: bool,
        shift: bool,
        command: bool,
    ) {
        let chars: Vec<char> = name.chars().collect();
        if chars.len() == 1 {
            let key = Key::_Raw(chars[0] as _);
            self._input_key(key, down, press, alt, ctrl, shift, command);
        } else {
            if let Some(key) = KEY_MAP.get(name) {
                self._input_key(key.clone(), down, press, alt, ctrl, shift, command);
            }
        }
    }

    // flutter only TODO new input
    pub fn input_string(&self, value: &str) {
        let mut key_event = KeyEvent::new();
        key_event.set_seq(value.to_owned());
        let mut msg_out = Message::new();
        msg_out.set_key_event(key_event);
        self.send(Data::Message(msg_out));
    }

    pub fn handle_flutter_key_event(
        &self,
        _name: &str,
        keycode: i32,
        scancode: i32,
        lock_modes: i32,
        down_or_up: bool,
    ) {
        if scancode < 0 || keycode < 0 {
            return;
        }
        let keycode: u32 = keycode as u32;
        let scancode: u32 = scancode as u32;

        #[cfg(not(target_os = "windows"))]
        let key = rdev::key_from_code(keycode) as rdev::Key;
        // Windows requires special handling
        #[cfg(target_os = "windows")]
        let key = rdev::get_win_key(keycode, scancode);

        let event_type = if down_or_up {
            EventType::KeyPress(key)
        } else {
            EventType::KeyRelease(key)
        };
        let event = Event {
            time: SystemTime::now(),
            unicode: None,
            code: keycode as _,
            scan_code: scancode as _,
            event_type: event_type,
        };
        keyboard::client::process_event(&event, Some(lock_modes), true);
    }

    // flutter only TODO new input
    fn _input_key(
        &self,
        key: Key,
        down: bool,
        press: bool,
        alt: bool,
        ctrl: bool,
        shift: bool,
        command: bool,
    ) {
        let v = if press {
            3
        } else if down {
            1
        } else {
            0
        };
        let mut key_event = KeyEvent::new();
        match key {
            Key::Chr(chr) => {
                key_event.set_chr(chr);
            }
            Key::ControlKey(key) => {
                key_event.set_control_key(key.clone());
            }
            Key::_Raw(raw) => {
                key_event.set_chr(raw);
            }
        }

        if v == 1 {
            key_event.down = true;
        } else if v == 3 {
            key_event.press = true;
        }
        keyboard::client::legacy_modifiers(&mut key_event, alt, ctrl, shift, command);
        key_event.mode = KeyboardMode::Legacy.into();

        self.send_key_event(&key_event);
    }

    pub fn send_key_event(&self, evt: &KeyEvent) {
        // mode: legacy(0), map(1), translate(2), auto(3)
        let mut msg_out = Message::new();
        msg_out.set_key_event(evt.clone());
        self.send(Data::Message(msg_out));
    }

    pub fn send_mouse(
        &self,
        mask: i32,
        x: i32,
        y: i32,
        alt: bool,
        ctrl: bool,
        shift: bool,
        command: bool,
    ) {
        #[allow(unused_mut)]
        let mut command = command;
        #[cfg(windows)]
        {
            if !command && crate::platform::windows::get_win_key_state() {
                command = true;
            }
        }

        // #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let (alt, ctrl, shift, command) =
            keyboard::client::get_modifiers_state(alt, ctrl, shift, command);

        self.to_mouse_event(mask, x, y, alt, ctrl, shift, command);

        // on macos, ctrl + left button down = right button down, up won't emit, so we need to
        // emit up myself if peer is not macos
        // to-do: how about ctrl + left from win to macos
        if cfg!(target_os = "macos") {
            let buttons = mask >> 3;
            let evt_type = mask & 0x7;
            if buttons == 1 && evt_type == 1 && ctrl && whoami::platform().to_string() != "Mac OS" {
                self.send_mouse((1 << 3 | 2) as _, x, y, alt, ctrl, shift, command);
            }
        }
    }

    /// Send mouse data.
    ///
    /// # Arguments
    ///
    /// * `mask` - Mouse event.
    ///     * mask = buttons << 3 | type
    ///     * type, 1: down, 2: up, 3: wheel, 4: trackpad
    ///     * buttons, 1: left, 2: right, 4: middle
    /// * `x` - X coordinate.
    /// * `y` - Y coordinate.
    /// * `alt` - Whether the alt key is pressed.
    /// * `ctrl` - Whether the ctrl key is pressed.
    /// * `shift` - Whether the shift key is pressed.
    /// * `command` - Whether the command key is pressed.
    /// * `interface` - The interface for sending data.
    #[inline]
    pub fn to_mouse_event(
        &self,
        mask: i32,
        x: i32,
        y: i32,
        alt: bool,
        ctrl: bool,
        shift: bool,
        command: bool,
    ) {
        let mut msg_out = Message::new();
        let mut mouse_event = MouseEvent {
            mask,
            x,
            y,
            ..Default::default()
        };
        if alt {
            mouse_event.modifiers.push(ControlKey::Alt.into());
        }
        if shift {
            mouse_event.modifiers.push(ControlKey::Shift.into());
        }
        if ctrl {
            mouse_event.modifiers.push(ControlKey::Control.into());
        }
        if command {
            mouse_event.modifiers.push(ControlKey::Meta.into());
        }
        #[cfg(all(target_os = "macos"))]
        if check_scroll_on_mac(mask, x, y) {
            mouse_event.modifiers.push(ControlKey::Scroll.into());
        }
        msg_out.set_mouse_event(mouse_event);
        self.send(Data::Message(msg_out));
    }

    pub fn reconnect(
        &self,
        force_relay: bool,
        mut rx_to_local_stream: mpsc::UnboundedReceiver<Message>,
        tx_from_local_stream: mpsc::UnboundedSender<Message>,
    ) {
        // self.send(Data::Close);
        let cloned = self.clone();
        // override only if true
        // if true == force_relay {
        //     cloned.lc.write().unwrap().force_relay = true;
        // }
        let mut lock = self.thread.lock().unwrap();
        lock.take().map(|t| t.join());
        *lock = Some(std::thread::spawn(move || {
            io_loop(cloned, rx_to_local_stream, tx_from_local_stream);
        }));
    }

    fn on_connected(&self) {
        crate::keyboard::client::start_grab_loop(true);
        self.enter()
    }
}

#[tokio::main(flavor = "current_thread")]
pub async fn io_loop<T: InvokeUiSession>(
    handler: Bot<T>,
    mut rx_to_local_stream: mpsc::UnboundedReceiver<Message>,
    tx_from_local_stream: mpsc::UnboundedSender<Message>,
) {
    // It is ok to call this function multiple times.
    #[cfg(target_os = "windows")]
    if !handler.is_file_transfer() && !handler.is_port_forward() {
        clipboard::ContextSend::enable(true);
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    let (sender, receiver) = mpsc::unbounded_channel::<Data>();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let (sender, mut receiver) = mpsc::unbounded_channel::<Data>();
    *handler.sender.write().unwrap() = Some(sender.clone());

    // let token = LocalConfig::get_option("access_token");
    // let key = crate::get_key(false).await;
    // #[cfg(not(any(target_os = "android", target_os = "ios")))]
    // if handler.is_port_forward() {
    //     if handler.is_rdp() {
    //         let port = handler
    //             .get_option("rdp_port".to_owned())
    //             .parse::<i32>()
    //             .unwrap_or(3389);
    //         std::env::set_var(
    //             "rdp_username",
    //             handler.get_option("rdp_username".to_owned()),
    //         );
    //         std::env::set_var(
    //             "rdp_password",
    //             handler.get_option("rdp_password".to_owned()),
    //         );
    //         log::info!("Remote rdp port: {}", port);
    //         start_one_port_forward(handler, 0, "".to_owned(), port, receiver, &key, &token).await;
    //     } else if handler.args.len() == 0 {
    //         let pfs = handler.lc.read().unwrap().port_forwards.clone();
    //         let mut queues = HashMap::<i32, mpsc::UnboundedSender<Data>>::new();
    //         for d in pfs {
    //             sender.send(Data::AddPortForward(d)).ok();
    //         }
    //         loop {
    //             match receiver.recv().await {
    //                 Some(Data::AddPortForward((port, remote_host, remote_port))) => {
    //                     if port <= 0 || remote_port <= 0 {
    //                         continue;
    //                     }
    //                     let (sender, receiver) = mpsc::unbounded_channel::<Data>();
    //                     queues.insert(port, sender);
    //                     let handler = handler.clone();
    //                     let key = key.clone();
    //                     let token = token.clone();
    //                     tokio::spawn(async move {
    //                         start_one_port_forward(
    //                             handler,
    //                             port,
    //                             remote_host,
    //                             remote_port,
    //                             receiver,
    //                             &key,
    //                             &token,
    //                         )
    //                         .await;
    //                     });
    //                 }
    //                 Some(Data::RemovePortForward(port)) => {
    //                     if let Some(s) = queues.remove(&port) {
    //                         s.send(Data::Close).ok();
    //                     }
    //                 }
    //                 Some(Data::Close) => {
    //                     break;
    //                 }
    //                 Some(d) => {
    //                     for (_, s) in queues.iter() {
    //                         s.send(d.clone()).ok();
    //                     }
    //                 }
    //                 _ => {}
    //             }
    //         }
    //     } else {
    //         let port = handler.args[0].parse::<i32>().unwrap_or(0);
    //         if handler.args.len() != 3
    //             || handler.args[2].parse::<i32>().unwrap_or(0) <= 0
    //             || port <= 0
    //         {
    //             handler.on_error("Invalid arguments, usage:<br><br> rustdesk --port-forward remote-id listen-port remote-host remote-port");
    //         }
    //         let remote_host = handler.args[1].clone();
    //         let remote_port = handler.args[2].parse::<i32>().unwrap_or(0);
    //         start_one_port_forward(
    //             handler,
    //             port,
    //             remote_host,
    //             remote_port,
    //             receiver,
    //             &key,
    //             &token,
    //         )
    //         .await;
    //     }
    //     return;
    // }

    let frame_count = Arc::new(AtomicUsize::new(0));
    let frame_count_cl = frame_count.clone();
    let ui_handler = handler.ui_handler.clone();
    let (video_sender, audio_sender, video_queue, decode_fps) =
        start_video_audio_threads(move |data: &mut scrap::ImageRgb| {
            frame_count_cl.fetch_add(1, Ordering::Relaxed);
            ui_handler.on_rgba(data);
        });

    let mut new_macro = Macro::new(
        handler,
        video_queue,
        video_sender,
        audio_sender,
        receiver,
        sender,
        frame_count,
        decode_fps,
    );
    new_macro
        .io_loop(rx_to_local_stream, tx_from_local_stream)
        .await;
    // new_macro.sync_jobs_status_to_local().await;

    // let mut remote = Remote::new(
    //     handler,
    //     video_queue,
    //     video_sender,
    //     audio_sender,
    //     receiver,
    //     sender,
    //     frame_count,
    //     decode_fps,
    // );
    // remote.io_loop(&key, &token).await;
    // remote.sync_jobs_status_to_local().await;
}

#[derive(Clone, Default)]
pub struct Rect {
    x0: u8,
    y0: u8,
    x1: u8,
    y1: u8,
}

#[derive(Clone, Default)]
pub struct StepEvent {
    time: Duration,
    message_input: MessageInput,
    screen: Vec<u8>,
    mouse_pos: (u8, u8),
    win_info: (String, String, Rect),
}

#[derive(Clone, Default)]
pub enum BotSync {
    Sync,
    #[default]
    Forward,
}

lazy_static::lazy_static! {
    static ref STEP_EVENT: Arc::<Mutex<StepEvent>> = Default::default();
    pub static ref CUR_BOT: Arc<Mutex<Option<Bot<crate::ui::remote::SciterHandler>>>> = Default::default();
}

pub struct Macro<T: InvokeUiSession> {
    handler: Bot<T>,
    video_sender: MediaSender,
    audio_sender: MediaSender,
    receiver: mpsc::UnboundedReceiver<Data>,
    sender: mpsc::UnboundedSender<Data>,
    old_clipboard: Arc<Mutex<String>>,
    timer: Interval,
    last_update_jobs_status: (Instant, HashMap<i32, u64>),
    first_frame: bool,
    #[cfg(windows)]
    client_conn_id: i32, // used for clipboard
    data_count: Arc<AtomicUsize>,
    frame_count: Arc<AtomicUsize>,
    video_format: CodecFormat,

    recording_macro: bool,
    enable_mouse_macro: bool,
    enable_kb_macro: bool,
    step_events: Vec<StepEvent>,
}

impl<T: InvokeUiSession> Macro<T> {
    pub fn new(
        handler: Bot<T>,
        video_sender: MediaSender,
        audio_sender: MediaSender,
        receiver: mpsc::UnboundedReceiver<Data>,
        sender: mpsc::UnboundedSender<Data>,
        frame_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            handler,
            video_sender,
            audio_sender,
            receiver,
            sender,
            old_clipboard: Default::default(),
            timer: time::interval(SEC30),
            last_update_jobs_status: (Instant::now(), Default::default()),
            first_frame: false,
            #[cfg(windows)]
            client_conn_id: 0,
            data_count: Arc::new(AtomicUsize::new(0)),
            frame_count,
            video_format: CodecFormat::Unknown,
            recording_macro: Default::default(),
            enable_mouse_macro: Default::default(),
            enable_kb_macro: Default::default(),
            step_events: vec![],
        }
    }

    pub async fn io_loop(
        &mut self,
        mut rx_to_local_stream: mpsc::UnboundedReceiver<Message>,
        tx_from_local_stream: mpsc::UnboundedSender<Message>,
    ) {
        // receiver - event listener from UI, in my case its Tauri
        // sender - event sender to UI, in my case its Tauri

        // peer - remote event listener, in my case its turn ON from sender event, by default its OFF

        // TODO: Event listener using tauri
        // local - local event listener, in my case its turn ON from receiver event, by default its OFF

        // TODO:
        // tcp connection to server
        let mut last_recv_time = Instant::now();
        let mut received = false;
        let conn_type = if self.handler.is_file_transfer() {
            ConnType::FILE_TRANSFER
        } else {
            ConnType::default()
        };

        // self.handler.set_connection_type(peer.is_secured(), direct); // flutter -> connection_ready
        // self.handler.set_connection_info(direct, false);

        // just build for now
        #[cfg(not(windows))]
        let (_tx_holder, mut rx_clip_client) = mpsc::unbounded_channel::<i32>();

        #[cfg(windows)]
        let (_tx_holder, rx) = mpsc::unbounded_channel();
        #[cfg(windows)]
        let mut rx_clip_client_lock = Arc::new(TokioMutex::new(rx));
        #[cfg(windows)]
        {
            let is_conn_not_default = self.handler.is_file_transfer()
                || self.handler.is_port_forward()
                || self.handler.is_rdp();
            if !is_conn_not_default {
                (self.client_conn_id, rx_clip_client_lock) = clipboard::get_rx_cliprdr_client("");
                //&self.handler.id);
            };
        }
        #[cfg(windows)]
        let mut rx_clip_client = rx_clip_client_lock.lock().await;

        let mut status_timer = time::interval(Duration::new(1, 0));

        loop {
            let mut start = Instant::now();
            tokio::select! {
                res = rx_to_local_stream.recv() => {
                    if let Some(res) = res {
                        let msg = res.clone();
                        match msg.union {
                            Some(message::Union::TestDelay(msg)) => {
                                // log::info!("rx_to_local_stream {:#?}", msg);
                            }
                            Some(message::Union::VideoFrame(msg)) => {
                                // TODO: update latest frame temp
                                // log::info!("rx_to_local_stream {:#?}", "msg.union.VideoFrame");
                            }
                            Some(message::Union::CursorData(msg)) => {
                                let mut step_event = STEP_EVENT.lock().unwrap();

                                // *step_event = StepEvent{
                                //     time: start.elapsed(),
                                //     message_input: todo!(),
                                //     screen: todo!(),
                                //     mouse_pos: todo!(),
                                //     win_info: todo!()
                                // };
                                start = Instant::now();
                                log::info!("rx_to_local_stream CursorData {:#?}", msg);
                            }
                            Some(message::Union::AudioFrame(msg)) => {
                                // log::info!("rx_to_local_stream {:#?}", "msg.union.AudioFrame");
                            }
                            _ => {
                                log::info!("rx_to_local_stream {:#?}", msg.union);
                            }
                        }

                        last_recv_time = Instant::now();
                        if !received {
                            received = true;
                            // self.handler.set_connection_info(direct, true);
                        }
                        // self.data_count.fetch_add(res.len(), Ordering::Relaxed);
                        // if let Err(_) = tokio::time::timeout(Duration::from_secs(10), async {
                        //     // net::TcpStream::connect((host, port)).is_ok()
                        //    let temp =  self.handle_msg_from_local(res, &tx_from_local_stream).await;
                        // })
                        // .await {
                        //     println!("did not receive value within 10 ms");
                        // };
                        if !self.handle_msg_from_local(res, &tx_from_local_stream).await {
                            break
                        }
                    } else {
                        if self.handler.is_restarting_remote_device() {
                            log::info!("Restart remote device");
                            self.handler.msgbox("restarting", "Restarting Remote Device", "remote_restarting_tip", "");
                        } else {
                            log::info!("Reset by the peer");
                            self.handler.msgbox("error", "Connection Error", "Reset by the peer", "");
                        }
                        break;
                    }
                }
                data = self.receiver.recv() => {
                    if let Some(data) = data {
                        if !self.handle_msg_from_ui(data, &tx_from_local_stream).await {
                            break;
                        }
                    }
                }
                _msg = rx_clip_client.recv() => {
                    #[cfg(windows)]
                    match _msg {
                        Some(clip) => {
                            allow_err!(tx_from_local_stream.send(crate::clipboard_file::clip_2_msg(clip)));
                        }
                        None => {
                            // unreachable!()
                        }
                    }
                }
                _ = self.timer.tick() => {
                    if last_recv_time.elapsed() >= SEC30 {
                        self.handler.msgbox("error", "Connection Error", "Timeout", "");
                        break;
                    }
                    self.timer = time::interval_at(Instant::now() + SEC30, SEC30);
                }
                _ = status_timer.tick() => {
                    let speed = self.data_count.swap(0, Ordering::Relaxed);
                    let speed = format!("{:.2}kB/s", speed as f32 / 1024 as f32);
                    // let fps = self.frame_count.swap(0, Ordering::Relaxed) as _;
                    // self.handler.update_quality_status(QualityStatus {
                    //     speed:Some(speed),
                    //     fps:Some(fps),
                    //     ..Default::default()
                    // });
                }
            }
        }
        log::debug!("Exit io_loop of id={}", self.handler.id);
        // Stop client audio server.
        // if let Some(s) = self.stop_voice_call_sender.take() {
        //     s.send(()).ok();
        // }
    }
    // """
    // start/stop recording a macro
    // """
    fn macro_start_stop(&mut self) {
        if self.recording_macro {
            self.recording_macro = false;
        } else {
            // self.initial_position = self.get_mouse_position();
            self.enable_mouse_macro = true;
            self.enable_kb_macro = true;
            self.step_events = Vec::new();
            self.recording_macro = true;
        }
    }

    fn events_to_steps(&self) {}

    async fn handle_msg_from_local(
        &mut self,
        msg_in: Message,
        tx_from_local_stream: &mpsc::UnboundedSender<Message>,
    ) -> bool {
        match msg_in.union {
            Some(message::Union::Hash(hash)) => {
                // self.handler
                //     .handle_hash(&self.handler.password.clone(), hash, peer)
                //     .await;
                let mut option = Some(OptionMessage::default());

                let mut pr = ProcessingRequest {
                    username: Config::get_id(),
                    password: Vec::new().into(),
                    my_id: "007".to_owned(),
                    my_name: crate::username(),
                    option: option.into(),
                    session_id: 1,
                    version: crate::VERSION.to_string(),
                    ..Default::default()
                };

                let mut msg_out = Message::new();
                msg_out.set_processing_request(pr);
                tx_from_local_stream.send(msg_out).ok();
            }
            Some(message::Union::VideoFrame(vf)) => {
                if !self.first_frame {
                    self.first_frame = true;
                    // self.handler.close_success();
                    // self.handler.adapt_size();
                    // self.send_opts_after_login(peer).await;
                }
                let incomming_format = CodecFormat::from(&vf);
                if self.video_format != incomming_format {
                    self.video_format = incomming_format.clone();
                    // self.handler.update_quality_status(QualityStatus {
                    //     codec_format: Some(incomming_format),
                    //     ..Default::default()
                    // })
                };
                self.video_sender.send(MediaData::VideoFrame(vf)).ok();
            }
            Some(message::Union::LoginResponse(lr)) => match lr.union {
                Some(login_response::Union::Error(err)) => {
                    // if !self.handler.handle_login_error(&err) {
                    //     return false;
                    // }
                }
                Some(login_response::Union::PeerInfo(pi)) => {
                    self.handler.handle_peer_info(pi);
                    self.check_clipboard_file_context();
                    if !(
                        self.handler.is_file_transfer() || self.handler.is_port_forward()
                        // || !SERVER_CLIPBOARD_ENABLED.load(Ordering::SeqCst)
                        // || !SERVER_KEYBOARD_ENABLED.load(Ordering::SeqCst)
                        // || self.handler.lc.read().unwrap().disable_clipboard
                    ) {
                        let txt = self.old_clipboard.lock().unwrap().clone();
                        if !txt.is_empty() {
                            let msg_out = crate::create_clipboard_msg(txt);
                            let sender = self.sender.clone();
                            tokio::spawn(async move {
                                // due to clipboard service interval time
                                sleep(common::CLIPBOARD_INTERVAL as f32 / 1_000.).await;
                                sender.send(Data::Message(msg_out)).ok();
                            });
                        }
                    }

                    // if self.handler.is_file_transfer() {
                    // self.handler.load_last_jobs();
                    // }
                }
                _ => {}
            },
            Some(message::Union::CursorData(cd)) => {
                // TODO: bot handler
                // self.handler.set_cursor_data(cd);
            }
            Some(message::Union::CursorId(id)) => {
                // self.handler.set_cursor_id(id.to_string());
            }
            Some(message::Union::CursorPosition(cp)) => {
                // self.handler.set_cursor_position(cp);
            }
            Some(message::Union::Clipboard(cb)) => {
                // if !self.handler.lc.read().unwrap().disable_clipboard {
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                update_clipboard(cb, Some(&self.old_clipboard));
                #[cfg(any(target_os = "android", target_os = "ios"))]
                {
                    let content = if cb.compress {
                        hbb_common::compress::decompress(&cb.content)
                    } else {
                        cb.content.into()
                    };
                    if let Ok(content) = String::from_utf8(content) {
                        self.handler.clipboard(content);
                    }
                }
                // }
            }
            #[cfg(windows)]
            Some(message::Union::Cliprdr(clip)) => {
                log::info!("cliprdr: {:#?}", clip);
                // self.handle_cliprdr_msg(clip);
            }
            Some(message::Union::Misc(misc)) => match misc.union {
                Some(misc::Union::AudioFormat(f)) => {
                    self.audio_sender.send(MediaData::AudioFormat(f)).ok();
                }
                Some(misc::Union::ChatMessage(c)) => {
                    // self.handler.new_message(c.text);
                }
                Some(misc::Union::PermissionInfo(p)) => {
                    log::info!("Change permission {:?} -> {}", p.permission, p.enabled);
                    match p.permission.enum_value_or_default() {
                        Permission::Keyboard => {
                            // SERVER_KEYBOARD_ENABLED.store(p.enabled, Ordering::SeqCst);
                            self.handler.set_permission("keyboard", p.enabled);
                        }
                        Permission::Clipboard => {
                            // SERVER_CLIPBOARD_ENABLED.store(p.enabled, Ordering::SeqCst);
                            self.handler.set_permission("clipboard", p.enabled);
                        }
                        Permission::Audio => {
                            self.handler.set_permission("audio", p.enabled);
                        }
                        Permission::File => {
                            // SERVER_FILE_TRANSFER_ENABLED.store(p.enabled, Ordering::SeqCst);
                            if !p.enabled && self.handler.is_file_transfer() {
                                return true;
                            }
                            self.check_clipboard_file_context();
                            self.handler.set_permission("file", p.enabled);
                        }
                        Permission::Restart => {
                            self.handler.set_permission("restart", p.enabled);
                        }
                        Permission::Recording => {
                            self.handler.set_permission("recording", p.enabled);
                        }
                    }
                }
                Some(misc::Union::SwitchDisplay(s)) => {
                    self.handler.switch_display(&s);
                    self.video_sender.send(MediaData::Reset).ok();
                    if s.width > 0 && s.height > 0 {
                        self.handler
                            .set_display(s.x, s.y, s.width, s.height, s.cursor_embedded);
                    }
                }
                Some(misc::Union::CloseReason(c)) => {
                    self.handler.msgbox("error", "Connection Error", &c, "");
                    return false;
                }
                Some(misc::Union::BackNotification(notification)) => {
                    if !self.handle_back_notification(notification).await {
                        return false;
                    }
                }
                Some(misc::Union::Uac(uac)) => {
                    let msgtype = "custom-uac-nocancel";
                    let title = "Prompt";
                    let text = "Please wait for confirmation of UAC...";
                    let link = "";
                    if uac {
                        self.handler.msgbox(msgtype, title, text, link);
                    } else {
                        self.handler
                            .cancel_msgbox(&format!("{}-{}-{}-{}", msgtype, title, text, link,));
                    }
                }
                Some(misc::Union::ForegroundWindowElevated(elevated)) => {
                    let msgtype = "custom-elevated-foreground-nocancel";
                    let title = "Prompt";
                    let text = "elevated_foreground_window_tip";
                    let link = "";
                    if elevated {
                        self.handler.msgbox(msgtype, title, text, link);
                    } else {
                        self.handler
                            .cancel_msgbox(&format!("{}-{}-{}-{}", msgtype, title, text, link,));
                    }
                }
                _ => {}
            },
            Some(message::Union::TestDelay(t)) => {
                self.handler
                    .handle_test_delay(t, tx_from_local_stream)
                    .await;
            }
            Some(message::Union::AudioFrame(frame)) => {
                // if !self.handler.lc.read().unwrap().disable_audio {
                self.audio_sender.send(MediaData::AudioFrame(frame)).ok();
                // }
            }
            Some(message::Union::MessageBox(msgbox)) => {
                let mut link = msgbox.link;
                if !link.starts_with("rustdesk://") {
                    if let Some(v) = hbb_common::config::HELPER_URL.get(&link as &str) {
                        link = v.to_string();
                    } else {
                        log::warn!("Message box ignore link {} for security", &link);
                        link = "".to_string();
                    }
                }
                self.handler
                    .msgbox(&msgbox.msgtype, &msgbox.title, &msgbox.text, &link);
            }
            _ => {}
        }
        true
    }

    async fn handle_msg_from_ui(
        &mut self,
        data: Data,
        tx_from_local_stream: &mpsc::UnboundedSender<Message>,
    ) -> bool {
        match data {
            Data::Close => {
                // let mut misc = Misc::new();
                // misc.set_close_reason("".to_owned());
                // let mut msg = Message::new();
                // msg.set_misc(misc);
                // allow_err!(peer.send(&msg).await);
                return false;
            }
            Data::Login((password, remember)) => {
                // self.handler
                //     .handle_login_from_ui(password, remember, peer)
                //     .await;
            }
            Data::ToggleClipboardFile => {
                self.check_clipboard_file_context();
            }
            Data::Message(msg) => {
                log::info!("Message {}", msg);
                // allow_err!(peer.send(&msg).await);
                tx_from_local_stream.send(msg);
            }
            Data::RecordScreen(start, w, h, id) => {
                // let _ = self
                //     .video_sender
                //     .send(MediaData::RecordScreen(start, w, h, id));
            }
            _ => {}
        }
        true
    }

    fn start_clipboard(&mut self) -> Option<std::sync::mpsc::Sender<()>> {
        if self.handler.is_file_transfer() || self.handler.is_port_forward() {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let old_clipboard = self.old_clipboard.clone();
        let tx_protobuf = self.sender.clone();
        // let lc = self.handler.lc.clone();
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        match ClipboardContext::new() {
            Ok(mut ctx) => {
                // ignore clipboard update before service start
                check_clipboard(&mut ctx, Some(&old_clipboard));
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(CLIPBOARD_INTERVAL));
                    match rx.try_recv() {
                        Ok(_) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            log::debug!("Exit clipboard service of client");
                            break;
                        }
                        _ => {}
                    }
                    // if !SERVER_CLIPBOARD_ENABLED.load(Ordering::SeqCst)
                    //     || !SERVER_KEYBOARD_ENABLED.load(Ordering::SeqCst)
                    //     // || lc.read().unwrap().disable_clipboard
                    // {
                    //     continue;
                    // }
                    if let Some(msg) = check_clipboard(&mut ctx, Some(&old_clipboard)) {
                        tx_protobuf.send(Data::Message(msg)).ok();
                    }
                });
            }
            Err(err) => {
                log::error!("Failed to start clipboard service of client: {}", err);
            }
        }
        Some(tx)
    }

    fn check_clipboard_file_context(&self) {
        #[cfg(windows)]
        {
            // let enabled = SERVER_FILE_TRANSFER_ENABLED.load(Ordering::SeqCst);
            // && self.handler.lc.read().unwrap().enable_file_transfer;
            // ContextSend::enable(enabled);
        }
    }

    async fn handle_back_notification(&mut self, notification: BackNotification) -> bool {
        match notification.union {
            Some(back_notification::Union::BlockInputState(state)) => {
                self.handle_back_msg_block_input(
                    state.enum_value_or(back_notification::BlockInputState::BlkStateUnknown),
                )
                .await;
            }
            Some(back_notification::Union::PrivacyModeState(state)) => {
                if !self
                    .handle_back_msg_privacy_mode(
                        state.enum_value_or(back_notification::PrivacyModeState::PrvStateUnknown),
                    )
                    .await
                {
                    return false;
                }
            }
            _ => {}
        }
        true
    }

    #[inline(always)]
    fn update_block_input_state(&mut self, on: bool) {
        // self.handler.update_block_input_state(on);
    }

    async fn handle_back_msg_block_input(&mut self, state: back_notification::BlockInputState) {
        match state {
            back_notification::BlockInputState::BlkOnSucceeded => {
                self.update_block_input_state(true);
            }
            back_notification::BlockInputState::BlkOnFailed => {
                self.handler
                    .msgbox("custom-error", "Block user input", "Failed", "");
                self.update_block_input_state(false);
            }
            back_notification::BlockInputState::BlkOffSucceeded => {
                self.update_block_input_state(false);
            }
            back_notification::BlockInputState::BlkOffFailed => {
                self.handler
                    .msgbox("custom-error", "Unblock user input", "Failed", "");
            }
            _ => {}
        }
    }

    #[inline(always)]
    fn update_privacy_mode(&mut self, on: bool) {
        // let mut config = self.handler.load_config();
        // config.privacy_mode = on;
        // self.handler.save_config(config);

        // self.handler.update_privacy_mode();
    }

    async fn handle_back_msg_privacy_mode(
        &mut self,
        state: back_notification::PrivacyModeState,
    ) -> bool {
        match state {
            back_notification::PrivacyModeState::PrvOnByOther => {
                self.handler.msgbox(
                    "error",
                    "Connecting...",
                    "Someone turns on privacy mode, exit",
                    "",
                );
                return false;
            }
            back_notification::PrivacyModeState::PrvNotSupported => {
                self.handler
                    .msgbox("custom-error", "Privacy mode", "Unsupported", "");
                self.update_privacy_mode(false);
            }
            back_notification::PrivacyModeState::PrvOnSucceeded => {
                self.handler
                    .msgbox("custom-nocancel", "Privacy mode", "In privacy mode", "");
                self.update_privacy_mode(true);
            }
            back_notification::PrivacyModeState::PrvOnFailedDenied => {
                self.handler
                    .msgbox("custom-error", "Privacy mode", "Peer denied", "");
                self.update_privacy_mode(false);
            }
            back_notification::PrivacyModeState::PrvOnFailedPlugin => {
                self.handler
                    .msgbox("custom-error", "Privacy mode", "Please install plugins", "");
                self.update_privacy_mode(false);
            }
            back_notification::PrivacyModeState::PrvOnFailed => {
                self.handler
                    .msgbox("custom-error", "Privacy mode", "Failed", "");
                self.update_privacy_mode(false);
            }
            back_notification::PrivacyModeState::PrvOffSucceeded => {
                self.handler
                    .msgbox("custom-nocancel", "Privacy mode", "Out privacy mode", "");
                self.update_privacy_mode(false);
            }
            back_notification::PrivacyModeState::PrvOffByPeer => {
                self.handler
                    .msgbox("custom-error", "Privacy mode", "Peer exit", "");
                self.update_privacy_mode(false);
            }
            back_notification::PrivacyModeState::PrvOffFailed => {
                self.handler
                    .msgbox("custom-error", "Privacy mode", "Failed to turn off", "");
            }
            back_notification::PrivacyModeState::PrvOffUnknown => {
                self.handler
                    .msgbox("custom-error", "Privacy mode", "Turned off", "");
                // log::error!("Privacy mode is turned off with unknown reason");
                self.update_privacy_mode(false);
            }
            _ => {}
        }
        true
    }
}
