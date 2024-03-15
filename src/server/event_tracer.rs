use std::{
    sync::mpsc::Receiver,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{clipboard_service::new, *};
use hbb_common::tokio::{sync::mpsc, sync::Mutex};
use image::{math, ImageBuffer, Rgb};
use scrap::{codec::Decoder, ImageFormat, ImageRgb};
use serde::{Serialize, Serializer, Deserialize};

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


#[derive(Serialize)]
struct EventRecord {
    timestamp: u128,
    mouse_x_pos: i32,
    mouse_y_pos: i32,
    event: String,
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

async fn save_state(frame: &Arc<Mutex<ImageRgb>>, mouse_pos: &(i32, i32), event_str: &String) {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let dirpath = format!("/tmp/{}/", time);

    let record = EventRecord {
        timestamp: time,
        mouse_x_pos: mouse_pos.0,
        mouse_y_pos: mouse_pos.1,
        event: event_str.to_owned(),
    };

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
    ).unwrap();

    let to_send = send_frame(&dirpath).await;
    if let Some(to_send) = to_send {
        resend_element_similarity(to_send).await;
    } else {
        println!("Nothing got from  ui_tracking")
    }
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
        match input {
            MessageInput::Mouse((mouse, id)) => {
                if (mouse.mask == 0) {
                    mouse_pos.0 = mouse.x;
                    mouse_pos.1 = mouse.y;
                } else {
                    save_state(&frame, &mouse_pos, &mouse.to_string()).await;
                }
            }
            MessageInput::Key((key, press)) => {
                save_state(&frame, &mouse_pos, &key.to_string()).await;
            }
            _ => {}
        };
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
