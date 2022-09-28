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
enum Channel {
    CountingStar(Vec<f64>),
    ImageProgress(usize),
}

#[allow(dead_code)]
enum Finalize {
    Data(Vec<u8>),
    Image(RetainedImage),
}

type TypedFlower = Flower<Channel, Finalize>;
type TypedFlowerHandle = Handle<Channel, Finalize>;

struct EframeTokioApp {
    rt: runtime::Runtime,
    flower: TypedFlower,
    init: bool,
    btn_label: String,
    net_image: NetworkImage,
    tmp_file_size: usize,
    seed: usize,
    show_image_progress: bool,
}

impl EframeTokioApp {
    fn new(_ctx: &CreationContext) -> Self {
        Self {
            rt: runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
            flower: TypedFlower::new(1),
            init: true,
            btn_label: "Cancel?".into(),
            net_image: Default::default(),
            tmp_file_size: 0,
            seed: 1,
            show_image_progress: true,
        }
    }

    fn show_init(&mut self) -> bool {
        let init = self.init;
        if self.init {
            self.init = false;
        }
        init
    }

    async fn fetch_image(url: String, handle: &TypedFlowerHandle) -> Result<Finalize, IOError> {
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
                    let progress = Channel::ImageProgress(a_chunk.len());
                    handle.send_async(progress).await;
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

            let finalize = Finalize::Image(retained_image);
            Ok(finalize)
        } else {
            Err(format!("Expected  image/jpeg png, found {}", content_type).into())
        }
    }

    fn spawn_fetch_image(&self, url: String) {
        let handle = self.flower.handle();
        self.rt.spawn(async move {
            // Don't forget to activate flower here
            handle.activate();
            let result = Self::fetch_image(url, &handle).await;
            // And set result
            handle.set_result(result);
        });
    }
}

impl eframe::App for EframeTokioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.show_init() {
                // Fetch image
                let url = format!("https://picsum.photos/seed/1/{}", REQ_IMAGE_SIZE);
                self.spawn_fetch_image(url);
                self.show_image_progress = true;
            }

            if self.flower.is_active() {
                self.flower
                    .extract(|channel| {
                        // Get Channel::ImageProgress since we only want usize value in this case.
                        if let Channel::ImageProgress(b) = channel {
                            self.tmp_file_size += b;
                        }
                    })
                    .finalize(|result| {
                        match result {
                            Ok(file_type) => {
                                // Get FileType::Image since we only want retained image in this case.
                                if let Finalize::Image(retained_image) = file_type {
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
                        self.show_image_progress = false;
                    });
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
                    self.spawn_fetch_image(url);
                    self.show_image_progress = true;
                }
            }

            if self.show_image_progress {
                ui.horizontal(|ui| {
                    // We don't need to call repaint since we are using spinner here.
                    ui.spinner();
                    let mut downloaded_size = self.tmp_file_size;
                    if downloaded_size > 0 {
                        // Convert current file size in Bytes to KB.
                        downloaded_size /= 1000;
                        // Show downloaded file size.
                        ui.label(format!("Downloaded size: {} KB", downloaded_size));
                    }
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
