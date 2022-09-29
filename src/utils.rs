use egui_extras::RetainedImage;

#[allow(dead_code)]
pub enum Message {
    CountingStar(Vec<f64>),
    ImageProgress(usize),
    ImageError,
    Default,
}

#[allow(dead_code)]
pub enum Container {
    Data(Vec<u8>),
    Image(RetainedImage),
}

#[derive(Default)]
pub struct NetworkImage {
    pub image: Option<RetainedImage>,
    pub file_size: usize,
    pub tmp_file_size: usize,
    pub show_image_progress: bool,
    pub error: Option<String>,
    pub seed: usize,
}

impl NetworkImage {
    pub fn set_image(&mut self, image: RetainedImage) {
        self.error.take();
        self.image = Some(image);
    }

    pub fn set_error(&mut self, e: impl ToString) {
        self.error = Some(e.to_string());
    }

    pub fn repair(&mut self) {
        // Convert final file size in Bytes to KB.
        if self.tmp_file_size >= 1000 {
            self.tmp_file_size /= 1000;
            self.file_size = self.tmp_file_size;
        }
        self.show_image_progress = false;
        self.tmp_file_size = 0;
    }
}
