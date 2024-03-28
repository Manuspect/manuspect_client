use std::{
    fmt::{self}, time::{SystemTime, UNIX_EPOCH}
};

use std::fmt::Display;

use winapi::um::{
    processthreadsapi::OpenProcess,
    psapi:: GetProcessImageFileNameW,
    winnt::PROCESS_ALL_ACCESS,
    winuser::{
        GetForegroundWindow, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, RealGetWindowClassW,
    },
};

use crate::input::*;

use super::*;
use hbb_common::tokio::{sync::mpsc, sync::Mutex};
use scrap::{codec::Decoder, ImageFormat, ImageRgb};
use serde::{Serialize, Deserialize};

use reqwest::{multipart, Body, Client};
use std::fs;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ElementsSimilarityElements {
    bbox: Vec<i32>,
    class_id: i32,
    id: i32,
    text: Option<String>
}
#[derive(Serialize, Deserialize, Clone, Debug)]
struct ElementsSimilarityBBoxes {
    class_id: i32,
    xc: f32,
    yc: f32,
    w: f32,
    h: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ElementsSimilarityRequest {
    session_id: String,
    screen_id: Option<String>,
    results: Vec<ElementsSimilarityElements>,
    bboxes: Vec<ElementsSimilarityBBoxes>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ElementsSimilarityResponse{
    session_id: String,
    screen_id: Option<String>,
    results: Vec<ElementsSimilarityElements>,
    bboxes: Vec<ElementsSimilarityBBoxes>
}


// #[derive(Serialize)]
// struct EventRecord {
//     timestamp: u128,
//     mouse_x_pos: i32,
//     mouse_y_pos: i32,
//     event: String,
// }

#[derive(Serialize)]
struct EventRecord {
    timestamp: u128,
    process_path: String,
    title: String,
    class_name: String,
    window_left: i32,
    window_top: i32,
    window_right: i32,
    window_bottom: i32,
    event: String,
    mouse_x_pos: i32,
    mouse_y_pos: i32,
    modifiers: String
}


fn decode_frame(decoder: &mut Decoder, frame_rgb: &mut ImageRgb, frame_msg: &Message) {
    match &frame_msg.union {
        Some(message::Union::VideoFrame(frame)) => {
            decoder.handle_video_frame(
                &frame.union.clone().unwrap(),
                frame_rgb,
                &mut std::ptr::null_mut(),
                &mut true,
                &mut None,
            );
        }
        _ => {}
    };
}

async fn save_state(frame: &Arc<Mutex<ImageRgb>>, mouse_pos: &(i32, i32), event_str: &String, modifiers: &String) {
    println!("save state ({};{}) - {}", mouse_pos.0, mouse_pos.1, event_str);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let dirpath = format!("/tmp/{}/", time);

    // #[cfg(target_os="Windows")]
    // TODO: only windows
    let hwnd = get_focus_hwnd();
    let title = get_title(hwnd);
    let rect = get_rect(hwnd);
    let process_id = get_process_id(hwnd);
    let class_name = get_class_name(hwnd);
    let process_path = get_process_path(hwnd);
    let record = EventRecord{
        timestamp: time,
        process_path: process_path,
        title: title,
        class_name: class_name,
        window_left: rect.0.left,
        window_top: rect.0.top,
        window_right: rect.0.right,
        window_bottom: rect.0.bottom,
        event: event_str.to_owned(),
        mouse_x_pos: mouse_pos.0,
        mouse_y_pos: mouse_pos.1,
        modifiers: modifiers.to_string()

    };

    // #[cfg(not(target_os="Windows"))]
    // let record = EventRecord {
    //     timestamp: time,
    //     mouse_x_pos: mouse_pos.0,
    //     mouse_y_pos: mouse_pos.1,
    //     event: event_str.to_owned(),
    // };

    let json_str = serde_json::to_string(&record).unwrap();

    tokio::fs::create_dir_all(&dirpath).await.unwrap();

    let json_path = format!("{}data.json", &dirpath);

    if let Err(err) = tokio::fs::write(json_path, &json_str).await {
        eprintln!("Error writing to file: {}", err);
    }

    let frame_path = format!("{}frame.png", &dirpath);

    let mut frame = frame.lock().await;
    image::save_buffer(
        frame_path,
        &frame.raw,
        frame.w as u32,
        frame.h as u32,
        image::ColorType::Rgba8,
    ).expect("may fail");

    // let to_send = send_frame(&dirpath).await;
    // if let Some(to_send) = to_send {
    //     resend_element_similarity(to_send).await;
    // } else {
    //     println!("Nothing got from  ui_tracking")
    // }
}

async fn resend_element_similarity(to_send: String) {
    // TODO: change host
    let url = "http://95.165.88.39:9000/element_similarity";


    let client = Client::new();
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "reqwest")
        .body(Body::from(to_send))
        .send()
        .await;

     // debug:
     match response {
        Ok(response) => {
            println!("{:?}", response);
            let text_response = response.text().await.unwrap();
            print!("{}", text_response);

        }
        Err(e) => {
            println!("{:?}", e);
        }  
    }
}

async fn send_frame(dir_path: &String) -> Option<String> {
    // TODO: change host
    let url = "http://95.165.88.39:9000/element_similarity";

    let file_name = "frame.png";
    let file_path = format!("{}{}", &dir_path, &file_name);
    let file_fs = fs::read(file_path).unwrap();

    let part = multipart::Part::bytes(file_fs).file_name(file_name);
    let form = reqwest::multipart::Form::new()
        .text("parent_id", "123")
        .text("session_id", "321")
        .part("screenshot", part);

    let content_type =  format!("multipart/form-data; boundary=\"{}\"", form.boundary());

    let client = Client::new();
    let response = client
        .post(url)
        .header("Content-Type", content_type)
        .header("User-Agent", "reqwest")
        .multipart(form)
        .send()
        .await;

    // debug:
    match response {
        Ok(response) => {
            println!("{:?}", response);
            let text_response = response.text().await.unwrap();
            print!("{}", text_response);
            return Some(text_response);

        }
        Err(e) => {
            println!("{:?}", e);
            return None;
        }  
    }
}

async fn handle_event(
    rx_events: &mut mpsc::UnboundedReceiver<MessageInput>,
    frame: &Arc<Mutex<ImageRgb>>,
    mouse_pos: &mut (i32, i32),
) {
    if let Some(input) = rx_events.recv().await {
        println!("got event on server side");
        match input {
            MessageInput::Mouse((mouse, id)) => {
                // if mouse.mask == 0 {
                    // mouse_pos.0 = mouse.x;
                    // mouse_pos.1 = mouse.y;
                    let event = get_mouse_event_from_mask(mouse.mask);
                    // let modifiers = Vec::new();
                    // for modifyer in mouse.modifiers {
                    //     modifiers.append(modifyer.enum_value())

                    // }
                // } else {
                    save_state(&frame, &(mouse.x, mouse.y), &event.to_string(), &format!("{:?}", mouse.modifiers)).await;
                // }
            }
            MessageInput::Key((key, press)) => {
                save_state(&frame, &mouse_pos, &key.to_string(), &format!("{:?}", key.modifiers)).await;
            }
            _ => {}
        };
    }
}


pub enum MouseLogEvents {
    LB_DOWN,
    LB_UP,
    RB_DOWN,
    RB_UP,
    MID_DOWN,
    MID_UP,
    UNDEFINED
}


impl fmt::Display for MouseLogEvents {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {

        let to_write = match self {
            MouseLogEvents::LB_DOWN =>  "LB_DOWN",
            MouseLogEvents::LB_UP =>      "LB_UP",
            MouseLogEvents::RB_DOWN =>  "RB_DOWN",
            MouseLogEvents::RB_UP =>      "RB_UP",
            MouseLogEvents::MID_DOWN =>"MID_DOWN",
            MouseLogEvents::MID_UP =>    "MID_UP",
            MouseLogEvents::UNDEFINED => "UNDEFINED"
        };
        write!(f, "{:?}", to_write)
        // or, alternatively:
        // fmt::Debug::fmt(self, f)
    }
}


fn get_mouse_event_from_mask(mask: i32) -> MouseLogEvents {
    
    type MLE = MouseLogEvents;

    let buttons = mask >> 3;
    let evt_type = mask & 0x7;

    match evt_type {
        // MOUSE_TYPE_MOVE => {
        //     en.mouse_move_to(evt.x, evt.y);
        //     *LATEST_PEER_INPUT_CURSOR.lock().unwrap() = Input {
        //         conn,
        //         time: get_time(),
        //         x: evt.x,
        //         y: evt.y,
        //     };
        // }
        MOUSE_TYPE_DOWN => match buttons {
            MOUSE_BUTTON_LEFT => MLE::LB_DOWN,
            MOUSE_BUTTON_RIGHT => MLE::RB_DOWN,
            MOUSE_BUTTON_WHEEL => MLE::MID_DOWN,
            // MOUSE_BUTTON_BACK => {
            //     allow_err!(en.mouse_down(MouseButton::Back));
            // }
            // MOUSE_BUTTON_FORWARD => {
            //     allow_err!(en.mouse_down(MouseButton::Forward));
            // }
            _ => MLE::UNDEFINED
        },
        MOUSE_TYPE_UP => match buttons {
            MOUSE_BUTTON_LEFT =>   MLE::LB_UP,
            MOUSE_BUTTON_RIGHT =>  MLE::RB_UP,
            MOUSE_BUTTON_WHEEL => MLE::MID_UP,

            // MOUSE_BUTTON_BACK => {
            //     en.mouse_up(MouseButton::Back);
            // }
            // MOUSE_BUTTON_FORWARD => {
            //     en.mouse_up(MouseButton::Forward);
            // }
            _ => MLE::UNDEFINED
        },
        // MOUSE_TYPE_WHEEL | MOUSE_TYPE_TRACKPAD => {
        //     #[allow(unused_mut)]
        //     let mut x = evt.x;
        //     #[allow(unused_mut)]
        //     let mut y = evt.y;
        //     #[cfg(not(windows))]
        //     {
        //         x = -x;
        //         y = -y;
        //     }

        //     #[cfg(any(target_os = "macos", target_os = "windows"))]
        //     let is_track_pad = evt_type == MOUSE_TYPE_TRACKPAD;

        //     #[cfg(target_os = "macos")]
        //     {
        //         // TODO: support track pad on win.

        //         // fix shift + scroll(down/up)
        //         if !is_track_pad
        //             && evt
        //                 .modifiers
        //                 .contains(&EnumOrUnknown::new(ControlKey::Shift))
        //         {
        //             x = y;
        //             y = 0;
        //         }

        //         if x != 0 {
        //             en.mouse_scroll_x(x, is_track_pad);
        //         }
        //         if y != 0 {
        //             en.mouse_scroll_y(y, is_track_pad);
        //         }
        //     }

        //     #[cfg(windows)]
        //     if !is_track_pad {
        //         x *= WHEEL_DELTA as i32;
        //         y *= WHEEL_DELTA as i32;
        //     }

        //     #[cfg(not(target_os = "macos"))]
        //     {
        //         if y != 0 {
        //             en.mouse_scroll_y(y);
        //         }
        //         if x != 0 {
        //             en.mouse_scroll_x(x);
        //         }
        //     }
        // }
        
        _ => MLE::UNDEFINED
    }
}


async fn handle_frame(
    rx_frames: &mut mpsc::UnboundedReceiver<Arc<Message>>,
    decoder: &mut Decoder,
    frame_rgb: &Arc<Mutex<ImageRgb>>,
) {
    if let Some(frame) = rx_frames.recv().await {
        let mut frame_rgb = frame_rgb.lock().await;
        decode_frame(decoder, &mut frame_rgb, &frame);
    }
}

pub fn trace(
    mut rx_frames: mpsc::UnboundedReceiver<Arc<Message>>,
    mut rx_events: mpsc::UnboundedReceiver<MessageInput>,
) {
    #[cfg(all(feature = "gpucodec", feature = "flutter"))]
    let luid = crate::flutter::get_adapter_luid();
    #[cfg(not(all(feature = "gpucodec", feature = "flutter")))]
    let luid = Default::default();
    log::info!("Starting tracing");

    let mut decoder = Decoder::new(luid);
    let rgb = ImageRgb::new(ImageFormat::ARGB, crate::DST_STRIDE_RGBA);

    let rgb_common = Arc::new(Mutex::new(rgb));
    let mut mouse_pos = (0, 0);

    tokio::spawn({
        let rgb_common = rgb_common.clone();
        async move {
            loop {
                handle_event(&mut rx_events, &rgb_common, &mut mouse_pos).await;
            }
        }
    });

    tokio::spawn({
        async move {
            loop {
                handle_frame(&mut rx_frames, &mut decoder, &rgb_common).await;
            }
        }
    });
}


/// Get the handle of the window that has the focus.
pub fn get_focus_hwnd() -> winapi::shared::windef::HWND {
    unsafe { GetForegroundWindow() }
}

/// Get the title of the window.
pub fn get_title(hwnd: winapi::shared::windef::HWND) -> String {
    let mut name: [u16; 256] = [0; 256];
    unsafe {
        GetWindowTextW(hwnd, name.as_mut_ptr() as *mut u16, 256);
    }
    String::from_utf16(&name).unwrap().trim_end_matches('\0').to_string()
}

/// Get the rectangle of the window.
pub fn get_rect(hwnd: winapi::shared::windef::HWND) -> RECT {
    let mut lp_rect = winapi::shared::windef::RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetWindowRect(hwnd, &mut lp_rect);
    }
    RECT(lp_rect)
}
 
/// Get the process ID of the window.
pub fn get_process_id(hwnd: winapi::shared::windef::HWND) -> u32 {
    let mut lpdw_process_id: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut lpdw_process_id) };
    lpdw_process_id
}

/// Get the class name of the window.
pub fn get_class_name(hwnd: winapi::shared::windef::HWND) -> String {
    let mut ptsz_class_name: [u16; 256] = [0; 256];
    unsafe {
        RealGetWindowClassW(hwnd, ptsz_class_name.as_mut_ptr(), 256);
    }
    String::from_utf16(&ptsz_class_name).unwrap().trim_end_matches('\0').to_string()
}

/// Get the path of the process.
pub fn get_process_path(hwnd: winapi::shared::windef::HWND) -> String {
    let process_id = get_process_id(hwnd);
    unsafe {
        let handle = OpenProcess(PROCESS_ALL_ACCESS, 0, process_id);

        let mut lpsz_file_name: [u16; 256] = [0; 256];
        GetProcessImageFileNameW(handle, lpsz_file_name.as_mut_ptr() as *mut u16, 256);

        String::from_utf16(&lpsz_file_name).unwrap().trim_end_matches('\0').to_string()
    }
}

/// Wrapper struct for the RECT type.
pub struct RECT(winapi::shared::windef::RECT);

impl Display for RECT {
    /// Formats the RECT struct as a string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rect = self.0;
        write!(
            f,
            "RECT {{ left: {}, top: {}, right: {}, bottom: {} }}",
            rect.left, rect.top, rect.right, rect.bottom
        )
    }
}

pub mod client {
    use crate::flutter::sessions;

    use super::*;


// use clap::Parser;
    // use print::println;
    use lazy_static::lazy_static;
    use rdev::{grab, Button, Event, EventType, Key, listen};
    // use screenshots::Screen;
    use serde::Serialize;
    // use std::any::Any;
    use std::collections::HashSet;
    // use std::process::id;
    // use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};

// use ::event_logger::EventLogger;


    lazy_static! {
        static ref KEY_BUFFER: Arc<Mutex<HashSet<Key>>> = Arc::new(Mutex::new(HashSet::new()));
        static ref CURRENT_MOUSE_POS: Arc<Mutex<(f64, f64)>> = Arc::new(Mutex::new((0.0, 0.0)));
        static ref EVENT_LOGGER: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));
        static ref SESSION_ID: Arc<Mutex<Option<uuid::Uuid>>> = Arc::new(Mutex::new(None));
    
    }

    pub fn start(session_id: uuid::Uuid) {

            *SESSION_ID.lock().unwrap() = Some(session_id);

            let event_logger = thread::spawn(|| {
                if let Err(error) = grab(event_handler) {
                    println!("Error: {:?}", error)
                }
            });
        
            // Store the event_logger thread handle
            *EVENT_LOGGER.lock().unwrap() = Some(event_logger);

            // wait for the event_logger thread to finish
            if let Some(event_logger) = EVENT_LOGGER.lock().unwrap().take() {
                event_logger.join().unwrap();
            }


    }

    fn stop_log() {
        println!("Stopping the event logger");

    }
                                                                    
    fn event_listener(event: Event) { 
        event_handler(event);
    }

    fn mouse_send(mouse_event: MouseEvent){

        let mut mask = mouse_event.mouse_type;
        
        // mask = match &mouse_event.mouse_type {
        //     "down" => MOUSE_TYPE_DOWN,
        //     "up" => MOUSE_TYPE_UP,
        //     "wheel" => MOUSE_TYPE_WHEEL,
        //     "trackpad" => MOUSE_TYPE_TRACKPAD,
        //     _ => 0,
        // };
        
       
        mask |= match mouse_event.button {
            Button::Left => MOUSE_BUTTON_LEFT,
            Button::Right => MOUSE_BUTTON_RIGHT,
            Button::Middle => MOUSE_BUTTON_WHEEL,
            // "back" => MOUSE_BUTTON_BACK,
            // "forward" => MOUSE_BUTTON_FORWARD,
            _ => 0,
        } << 3;
        
        let alt = mouse_event.modifiers.contains(&Key::Alt);
        let ctrl = mouse_event.modifiers.contains(&Key::ControlLeft);
        let shift = mouse_event.modifiers.contains(&Key::ShiftLeft);
        let command = mouse_event.modifiers.contains(&Key::MetaLeft);

        let session_id = SESSION_ID.lock().unwrap().unwrap();
        if let Some(session) = sessions::get_session_by_session_id(&session_id) {
            session.send_mouse(mask, mouse_event.x as i32, mouse_event.y as i32, alt, ctrl, shift, command);
        }
    }

    fn event_handler(event: Event) -> Option<Event> {

        match event.event_type {
            EventType::MouseMove { x: m_x, y: m_y } => {
                let ref mut current_pos = *CURRENT_MOUSE_POS.lock().unwrap();
                current_pos.0 = m_x;
                current_pos.1 = m_y;
                Some(event)
            }

            EventType::ButtonPress(button) => {
                let ref mut current_pos = *CURRENT_MOUSE_POS.lock().unwrap();

                let ref mut modifiers = *KEY_BUFFER.lock().unwrap();
                let mouse_event = MouseEvent {
                    x: current_pos.0.clone(),
                    y: current_pos.1.clone(),
                    button,
                    modifiers: modifiers.clone(),
                    mouse_type: MOUSE_TYPE_DOWN,
                };
                
                let json = serde_json::to_string(&mouse_event).unwrap();
                mouse_send(mouse_event);
                println!("saved json: {}", json);
                

                Some(event)
            }
                                        
            EventType::ButtonRelease(button) => {
                let ref mut current_pos = *CURRENT_MOUSE_POS.lock().unwrap();

                let ref mut modifiers = *KEY_BUFFER.lock().unwrap();
                let mouse_event = MouseEvent {
                    x: current_pos.0.clone(),
                    y: current_pos.1.clone(),
                    button,
                    modifiers: modifiers.clone(),
                    mouse_type: MOUSE_TYPE_UP
                };
                let json = serde_json::to_string(&mouse_event).unwrap();
                mouse_send(mouse_event);
            
                println!("saved json: {}", json);

                Some(event)
            }

            EventType::KeyPress(key) => {
                println!("Key pressed: {:?}", key);
                let mut buffer = KEY_BUFFER.lock().unwrap();
                buffer.insert(key);

                if key == Key::AltGr {
                    println!("Stopping the event logger");
                    stop_log()
                }
                Some(event)
            }
            EventType::KeyRelease(key) => {
                println!("Key released: {:?}", key);
                let mut buffer = KEY_BUFFER.lock().unwrap();
                buffer.remove(&key);
                Some(event)
            }
            EventType::Wheel {
                delta_x: _,
                delta_y: _,
            } => {
                let json = serde_json::to_string(&event).unwrap();
                println!("saved json: {}", json);
            
                Some(event)
            }
        }
    }
    #[derive(Clone, Debug, Serialize)]
    struct MouseEvent {
        x: f64,
        y: f64,
        button: Button,
        modifiers: HashSet<Key>,
        // TODO Enum
        mouse_type: i32
    } 
} 