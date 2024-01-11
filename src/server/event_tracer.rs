use std::sync::mpsc::Receiver;

use super::{clipboard_service::new, *};
use image::{ImageBuffer, Rgb};
use scrap::{codec::Decoder, ImageFormat, ImageRgb};

use hbb_common::{
    config::Config,
    fs,
    fs::can_enable_overwrite_detection,
    futures::{SinkExt, StreamExt},
    get_time, get_version_number,
    message_proto::{option_message::BoolOption, permission_info::Permission},
    password_security::{self as password, ApproveMode},
    sleep, timeout,
    tokio::{
        net::TcpStream,
        sync::mpsc,
        sync::Mutex,
        time::{self, Duration, Instant, Interval},
    },
    tokio_util::codec::{BytesCodec, Framed},
    toml::de,
};

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

async fn handle_event(
    rx_events: &mut mpsc::UnboundedReceiver<MessageInput>,
    frame: &Arc<Mutex<ImageRgb>>,
    cnt: &mut u32,
) {
    if let Some(input) = rx_events.recv().await {
        match input {
            MessageInput::Mouse((msg, id)) => {
                println!("mouse:{}", msg);
            }
            MessageInput::Key((msg, press)) => {
                println!("key:{}", msg);

                let filepath = format!("/tmp/frame{}.png", cnt);
                println!("{}", filepath);
                let mut frame_rgb = frame.lock().await;
                *cnt += 1;
                image::save_buffer(
                    filepath,
                    &frame_rgb.raw,
                    frame_rgb.w as u32,
                    frame_rgb.h as u32,
                    image::ColorType::Rgba8,
                );
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

    tokio::spawn({
        let rgb_common = rgb_common.clone();
        async move {
            let mut cnt: u32 = 0;
            loop {
                handle_event(&mut rx_events, &rgb_common, &mut cnt).await;
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
