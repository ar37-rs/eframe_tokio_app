use eframe::{egui, CreationContext};
use egui_extras::RetainedImage;
use flowync::{Flower, Handle, IntoResult, OIError};

// If download progress not shown (unnoticed due to internet connection too fast),
// try increase REQ_IMAGE_SIZE to 1024, 2048, 4096 or accordingly ...
const REQ_IMAGE_SIZE: usize = 512;

fn main() {
    let mut options = eframe::NativeOptions::default();
    options.always_on_top = true;
    eframe::run_native(
        "Download and show an image with eframe/egui",
        options,
        Box::new(|ctx| Box::new(EframeTokioApp::new(ctx))),
    );
}

#[derive(Clone)]
struct Data {
    debug_name: String,
    data: Vec<u8>,
}

impl Data {
    fn new(debug_name: impl ToString, data: Vec<u8>) -> Self {
        Self {
            debug_name: debug_name.to_string(),
            data,
        }
    }
}

#[derive(Default)]
struct NetworkImage {
    pub image: Option<RetainedImage>,
    pub file_size: usize,
    pub error: Option<String>,
}

impl NetworkImage {
    fn reset(&mut self) {
        self.error.take();
        self.image.take();
        self.file_size = 0;
    }

    fn set_image(&mut self, image: RetainedImage) {
        self.image = Some(image);
        let _ = self.error.take();
    }

    fn set_error(&mut self, error: impl ToString) {
        self.error = Some(error.to_string());
        let _ = self.image.take();
    }
}

type Flow = Flower<usize, Data>;
type FlowHandle = Handle<usize, Data>;

struct EframeTokioApp {
    rt: tokio::runtime::Runtime,
    flower: Flow,
    init: bool,
    fetching: bool,
    btn_label: String,
    net_image: NetworkImage,
    seed: usize,
}

impl EframeTokioApp {
    fn new(_ctx: &CreationContext) -> Self {
        Self {
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
            flower: Flow::new(1),
            fetching: true,
            init: true,
            btn_label: "Fetching...".into(),
            net_image: Default::default(),
            seed: 1,
        }
    }

    async fn reqwest_get(url: impl Into<String>, handle: &FlowHandle) -> Result<Data, OIError> {
        // Build a client
        let client = reqwest::Client::builder()
            // Needed to set UA to get image file, otherwise reqwest error 403
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:105.0) Gecko/20100101")
            .build()?;
        let mut response = client.get(url.into()).send().await?;

        // Get Content-Type
        let content_type = response
            .headers()
            .get("Content-Type")
            .catch("unable to get content type")?
            .to_str()?;

        if content_type.contains("image/jpeg") || content_type.contains("image/png") {
            let url = response.url().to_string();
            let vec_u8 = {
                let mut vec_u8 = Vec::new();
                while let Some(a_chunk) = response.chunk().await? {
                    // Send chunk size
                    handle.send(a_chunk.len());
                    a_chunk.into_iter().for_each(|x| {
                        vec_u8.push(x);
                    });
                }
                vec_u8
            };
            let data = Data::new(url, vec_u8);
            Ok(data)
        } else {
            Err(format!("Expected  image/jpeg png, found {}", content_type).into())
        }
    }

    fn fetch_image(&self, url: String) {
        let handle = self.flower.handle();
        self.rt.spawn(async move {
            handle.activate();
            match EframeTokioApp::reqwest_get(url, &handle).await {
                Ok(data) => handle.success(data),
                Err(e) => handle.error(e),
            }
        });
    }
}

impl eframe::App for EframeTokioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.init {
                self.init = false;
                // Fetch image
                let url = format!("https://picsum.photos/seed/1/{}", REQ_IMAGE_SIZE);
                self.fetch_image(url);
            }

            if ui.button(&self.btn_label).clicked() {
                if self.fetching {
                    self.btn_label = "Wait, are still fetching...".into();
                } else {
                    self.seed += 1;
                    self.btn_label = "Fetching...".into();
                    // Reset network image
                    self.net_image.reset();
                    // Refetch image
                    let url = format!(
                        "https://picsum.photos/seed/{}/{}",
                        self.seed, REQ_IMAGE_SIZE
                    );
                    self.fetch_image(url);
                    self.fetching = true;
                }
            }

            if self.fetching {
                // Request repaint to show progress.
                ctx.request_repaint();

                self.flower
                    .extract(|channel| {
                        if let Some(b) = channel {
                            // Add Bytes to the image file size.
                            self.net_image.file_size += b;
                        }

                        let mut downloaded_size = self.net_image.file_size;
                        if downloaded_size > 0 {
                            // Convert current file size in Bytes to KB.
                            downloaded_size /= 1000;
                            // Show downloaded file size.
                            ui.label(format!("Downloaded size: {} KB", downloaded_size));
                        } else {
                            ui.spinner();
                        }
                    })
                    .finalize(|result| {
                        match result {
                            Ok(image_data) => {
                                assert_eq!(self.net_image.file_size, image_data.data.len());
                                // Convert final file size in Bytes to KB.
                                self.net_image.file_size /= 1000;
                                match RetainedImage::from_image_bytes(
                                    image_data.debug_name,
                                    &image_data.data,
                                ) {
                                    Ok(image) => {
                                        self.net_image.set_image(image);
                                    }
                                    Err(err_msg) => self.net_image.set_error(err_msg),
                                }
                            }
                            Err(err_msg) => self.net_image.set_error(err_msg),
                        }
                        self.fetching = false;
                        self.btn_label = "Refetch image?".into();
                    });
            }

            if let Some(image) = &self.net_image.image {
                let file_size = self.net_image.file_size;
                ui.label(format!("File size: {} KB", file_size));
                ui.label(format!("Image size: {}x{} ", image.width(), image.height()));
                ui.label("Image URL:");
                let mut text = image.debug_name();
                let text_edit = egui::TextEdit::singleline(&mut text).desired_width(1000.0);
                ui.add(text_edit);

                egui::ScrollArea::both()
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        image.show_max_size(ui, image.size_vec2());
                    });
            }

            if let Some(err_msg) = &self.net_image.error {
                ui.colored_label(ui.visuals().error_fg_color, err_msg);
            }
        });
    }
}
