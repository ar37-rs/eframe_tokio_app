use eframe::{egui, CreationContext};
use egui_extras::RetainedImage;
use flowync::{Flower, Handle, IOError, IntoResult};
use reqwest::Client;
use tokio::runtime;
mod utils;
use utils::{Container, Message, NetworkImage};

const PPP: f32 = 1.25;

// If download progress not shown (unnoticed due to internet connection too fast),
// try increase REQ_IMAGE_SIZE to 1024, 2048 or between that accordingly, and
// if setted large than that may cause slow down at `image::from_image_bytes`,
// since we are on debug mode doing heavy iteraion is slow,
// and since we don't use parallelize image converting operation in that case.
const REQ_IMAGE_SIZE: usize = 512;

fn main() {
    let mut options = eframe::NativeOptions::default();
    options.always_on_top = true;
    eframe::run_native(
        "Eframe + Tokio integration example",
        options,
        Box::new(|ctx| Box::new(EframeTokioApp::new(ctx))),
    );
}

type TypedFlower = Flower<Message, Container>;
type TypedFlowerHandle = Handle<Message, Container>;

struct EframeTokioApp {
    rt: runtime::Runtime,
    flower: TypedFlower,
    init: bool,
    next_image: bool,
    btn_label_prev: String,
    btn_label_next: String,
    net_image: NetworkImage,
    error_msg: Message,
}

impl EframeTokioApp {
    fn new(ctx: &CreationContext) -> Self {
        ctx.egui_ctx.set_pixels_per_point(PPP);
        Self {
            rt: runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
            flower: TypedFlower::new(1),
            init: true,
            next_image: true,
            btn_label_prev: "Fetch prev image".into(),
            btn_label_next: "Fetch next image".into(),
            net_image: Default::default(),
            error_msg: Message::Default,
        }
    }

    fn show_init(&mut self) -> bool {
        let init = self.init;
        if self.init {
            self.init = false;
        }
        init
    }

    async fn fetch_image(url: String, handle: &TypedFlowerHandle) -> Result<Container, IOError> {
        // Build a client
        let client = Client::builder()
            // Needed to set UA to get image file, otherwise reqwest error 403
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:105.0) Gecko/20100101")
            .build()?;
        let mut response = client.get(url).send().await?;

        // Get Content-Type
        let content_type = response
            .headers()
            .get("Content-Type")
            .catch("unable to get content type")?
            .to_str()?;

        if content_type.contains("image/jpeg") || content_type.contains("image/png") {
            let debug_name = response.url().to_string();
            let cancelation_msg = "Fetching image canceled.";
            let mut image_bytes = Vec::new();
            {
                while let Some(a_chunk) = response.chunk().await? {
                    // Handle cancelation here
                    if handle.should_cancel() {
                        return Err(cancelation_msg.into());
                    }

                    // Send chunk size as download progress
                    let progress = Message::ImageProgress(a_chunk.len());
                    handle.send_async(progress).await;
                    a_chunk.into_iter().for_each(|x| {
                        image_bytes.push(x);
                    });
                }
            }

            let retained_image = RetainedImage::from_image_bytes(debug_name, &image_bytes)?;

            // And also handle cancelation here
            if handle.should_cancel() {
                return Err(cancelation_msg.into());
            }

            let finalize = Container::Image(retained_image);
            Ok(finalize)
        } else {
            Err(format!("Expected  image/jpeg png, found {}", content_type).into())
        }
    }

    fn spawn_fetch_image(&mut self, url: String) {
        // Set error to None
        self.net_image.error.take();
        // Show download image progress
        self.net_image.show_image_progress = true;
        // Get flower handle
        let handle = self.flower.handle();
        // Spawn tokio runtime.
        self.rt.spawn(async move {
            // Don't forget to activate flower here
            handle.activate();
            let fetch_image = Self::fetch_image(url, &handle).await;
            // Check if result is error
            if fetch_image.is_err() {
                // Blocking for a while here, it's fine because we are going to set the result ASAP anyway.
                handle.send(Message::ImageError);
            }
            // Set result
            handle.set_result(fetch_image);
        });
    }

    fn reset_fetch_image(&mut self) {
        // Handle logical accordingly
        self.net_image.repair();
        if self.next_image && self.flower.is_canceled() {
            if self.net_image.seed > 1 {
                self.net_image.seed -= 1;
            }
            self.btn_label_next = "Retry next image?".into();
        } else if !self.next_image && self.flower.is_canceled() {
            self.net_image.seed += 1;
            self.btn_label_prev = "Retry prev image?".into();
        } else {
            self.btn_label_next = "Fetch next image".into();
            self.btn_label_prev = "Fetch prev image".into();
        }
    }
}

impl eframe::App for EframeTokioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.show_init() {
                // Fetch image
                self.net_image.seed = 1;
                let url = format!("https://picsuxxm.photos/seed/1/{}", REQ_IMAGE_SIZE);
                self.spawn_fetch_image(url);
            }

            if self.flower.is_active() {
                let mut fetch_image_finalized = false;
                self.flower
                    .extract(|message| {
                        match message {
                            Message::ImageProgress(b) => {
                                self.net_image.tmp_file_size += b;
                            }
                            Message::DataProgress(_) => {
                                // Do stuff here if any
                            }
                            _ => {
                                // Set the error message if any.
                                self.error_msg = message;
                            }
                        }
                    })
                    .finalize(|result| {
                        match result {
                            // Get Container::Image since we only want retained image in this case.
                            Ok(Container::Image(retained_image)) => {
                                self.net_image.set_image(retained_image);
                                fetch_image_finalized = true;
                            }
                            // Handle if any
                            Ok(Container::Data(_data)) => {}
                            Err(err) => {
                                // Get specific error message.
                                match self.error_msg {
                                    Message::ImageError => {
                                        self.net_image.set_error(err);
                                        fetch_image_finalized = true;
                                    }
                                    Message::DataError => {
                                        // Handle DataError if any.
                                    }
                                    _ => eprintln!("{}", err),
                                }
                            }
                        }

                        // Set error message to default here.
                        self.error_msg = Message::Default;
                    });

                if fetch_image_finalized {
                    self.reset_fetch_image();
                }
            }

            ui.horizontal(|ui| {
                if ui.button(&self.btn_label_prev).clicked() {
                    if self.flower.is_active() {
                        if self.next_image {
                            self.btn_label_prev = "Wait we are still fetching...".into();
                        } else {
                            self.flower.cancel();
                        }
                    } else {
                        // Refetch prev image
                        if self.net_image.seed > 1 {
                            self.net_image.seed -= 1;
                            let url = format!(
                                "https://picsum.photos/seed/{}/{}",
                                self.net_image.seed, REQ_IMAGE_SIZE
                            );
                            self.spawn_fetch_image(url);
                            self.next_image = false;
                            self.btn_label_prev = "Cancel?".into();
                        } else {
                            self.btn_label_prev = "Prev image not available".into();
                        }
                    }
                }

                if ui.button(&self.btn_label_next).clicked() {
                    if self.flower.is_active() {
                        if !self.next_image {
                            self.btn_label_next = "Wait we are still fetching...".into();
                        } else {
                            self.flower.cancel();
                        }
                    } else {
                        // Refetch next image
                        self.net_image.seed += 1;
                        let url = format!(
                            "https://picsum.photos/seed/{}/{}",
                            self.net_image.seed, REQ_IMAGE_SIZE
                        );
                        self.spawn_fetch_image(url);
                        self.next_image = true;
                        self.btn_label_next = "Cancel?".into();
                    }
                }
            });

            if self.net_image.show_image_progress {
                ui.horizontal(|ui| {
                    // We don't need to call repaint since we are using spinner here.
                    ui.spinner();
                    let mut downloaded_size = self.net_image.tmp_file_size;
                    if downloaded_size > 0 {
                        // Convert current file size in Bytes to KB.
                        downloaded_size /= 1000;
                        // Show downloaded file size.
                        ui.label(format!("Downloaded size: {} KB", downloaded_size));
                    }
                });
            }

            if let Some(err) = &self.net_image.error {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }

            if let Some(image) = &self.net_image.image {
                let file_size = self.net_image.file_size;
                ui.label(format!("Current file size: {} KB", file_size));
                ui.label(format!(
                    "Current image size: {}x{} ",
                    image.width(),
                    image.height()
                ));
                ui.label("Current image URL:");
                let mut text = image.debug_name();
                let text_edit = egui::TextEdit::singleline(&mut text).desired_width(1000.0);
                ui.add(text_edit);

                egui::ScrollArea::both()
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        image.show_max_size(ui, image.size_vec2() / PPP);
                    });
            }
        });
    }
}
