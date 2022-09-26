use eframe::{egui, CreationContext};
use egui_extras::RetainedImage;
use flowync::{Flower, Handle, IOError, IntoResult};
use reqwest::Client;
use tokio::runtime;

// If download progress not shown (unnoticed due to internet connection too fast),
// try increase REQ_IMAGE_SIZE to 1024, 2048 or between that accordingly, and
// if setted large than that may cause slow down at `image::from_image_bytes`,
// since on debug mode doing heavy itertaion is slow,
// and since we don't use parallelize image converting operation in that case.
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

#[derive(Default)]
struct NetworkImage {
    pub image: Option<RetainedImage>,
    pub file_size: usize,
    pub error: Option<String>,
}

impl NetworkImage {
    fn set_image(&mut self, image: RetainedImage) {
        self.image = Some(image);
        let _ = self.error.take();
    }

    fn set_error(&mut self, error: impl ToString) {
        self.error = Some(error.to_string());
    }
}

#[allow(dead_code)]
enum FileType {
    Data(Vec<u8>),
    Image(RetainedImage),
}

type Flow = Flower<usize, FileType>;
type FlowHandle = Handle<usize, FileType>;

struct EframeTokioApp {
    rt: runtime::Runtime,
    flower: Flow,
    init: bool,
    btn_label: String,
    net_image: NetworkImage,
    tmp_file_size: usize,
    seed: usize,
}

impl EframeTokioApp {
    fn new(_ctx: &CreationContext) -> Self {
        Self {
            rt: runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
            flower: Flow::new(1),
            init: true,
            btn_label: "Cancel?".into(),
            net_image: Default::default(),
            tmp_file_size: 0,
            seed: 1,
        }
    }

    async fn reqwest_image(
        url: String,
        handle: &FlowHandle,
    ) -> Result<FileType, IOError> {
        // Build a client
        let client = Client::builder()
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
            let cancelation_msg = "Fetching other image canceled.";
            let vec_u8 = {
                let mut vec_u8 = Vec::new();
                while let Some(a_chunk) = response.chunk().await? {
                    // Handle cancelation here
                    if handle.should_cancel() {
                        return Err(cancelation_msg.into());
                    }

                    // Send chunk size as download progress
                    handle.send_async(a_chunk.len()).await;
                    a_chunk.into_iter().for_each(|x| {
                        vec_u8.push(x);
                    });
                }
                vec_u8
            };

            let retained_image = RetainedImage::from_image_bytes(url, &vec_u8)?;

            // And also handle cancelation here
            if handle.should_cancel() {
                return Err(cancelation_msg.into());
            }

            let file_type = FileType::Image(retained_image);
            Ok(file_type)
        } else {
            Err(format!("Expected  image/jpeg png, found {}", content_type).into())
        }
    }

    fn fetch_image(&self, url: String) {
        let handle = self.flower.handle();
        self.rt.spawn(async move {
            // Don't forget to activate flower here
            handle.activate();
            let result = EframeTokioApp::reqwest_image(url, &handle).await;
            // And set result
            handle.set_result(result);
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
                if self.flower.is_active() {
                    self.flower.cancel();
                } else {
                    // Set error to None
                    self.net_image.error.take();
                    // Refetch other image
                    self.seed += 1;
                    self.btn_label = "Cancel?".into();
                    let url = format!(
                        "https://picsum.photos/seed/{}/{}",
                        self.seed, REQ_IMAGE_SIZE
                    );
                    self.fetch_image(url);
                }
            }

            if self.flower.is_active() {
                // Request repaint to show progress.
                ctx.request_repaint();

                self.flower
                    .extract(|channel| {
                        if let Some(b) = channel {
                            // Add Bytes to the image file size.
                            self.tmp_file_size += b;
                        }

                        let mut downloaded_size = self.tmp_file_size;
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
                            Ok(file_type) => {
                                // Get FileType::Image since we only want retained image in this case.
                                if let FileType::Image(retained_image) = file_type {
                                    // Convert final file size in Bytes to KB.
                                    self.tmp_file_size /= 1000;
                                    self.net_image.set_image(retained_image);
                                    self.net_image.file_size = self.tmp_file_size;
                                }
                            }
                            Err(err_msg) => self.net_image.set_error(err_msg),
                        }
                        // Reset value if finalized
                        self.btn_label = "Refetch other image?".into();
                        self.tmp_file_size = 0;
                    });
            }

            if let Some(err_msg) = &self.net_image.error {
                ui.colored_label(ui.visuals().error_fg_color, err_msg);
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
        });
    }
}
