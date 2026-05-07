use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Style, Stylize},
    text::Line,
    widgets::{Block, Clear, Wrap},
};
use wiki_api::document::FigureData;

use crate::{
    action::{Action, ActionResult},
    config::{Config, PageImagesConfig, PageImagesEnabled, Theme},
    terminal::Frame,
    ui::centered_rect,
};

use super::Component;

#[cfg(feature = "image-support")]
use {
    image::DynamicImage,
    ratatui::layout::{Constraint, Direction, Layout},
    ratatui_image::{
        picker::{Picker, ProtocolType},
        protocol::StatefulProtocol,
        Resize, StatefulImage,
    },
    tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver},
    tracing::debug,
};

#[derive(Clone, Default)]
pub struct ImageSupport {
    #[cfg(feature = "image-support")]
    picker: Option<Picker>,
    reason: Option<String>,
}

impl ImageSupport {
    pub fn detect(config: &PageImagesConfig) -> Self {
        if config.enabled == PageImagesEnabled::Never {
            return Self {
                #[cfg(feature = "image-support")]
                picker: None,
                reason: Some("Image display is disabled in the page configuration".to_string()),
            };
        }

        Self::detect_inner()
    }

    #[cfg(feature = "image-support")]
    fn detect_inner() -> Self {
        match Picker::from_query_stdio() {
            Ok(picker) => {
                debug!(
                    "detected terminal image protocol: {:?}",
                    picker.protocol_type()
                );
                Self {
                    picker: Some(picker),
                    reason: None,
                }
            }
            Err(error) => Self {
                picker: None,
                reason: Some(format!("Terminal image support detection failed: {error}")),
            },
        }
    }

    #[cfg(not(feature = "image-support"))]
    fn detect_inner() -> Self {
        Self {
            reason: Some("wiki-tui was built without image support".to_string()),
        }
    }

    #[cfg(feature = "image-support")]
    fn picker(&self, config: &PageImagesConfig) -> Result<Picker, String> {
        if config.enabled == PageImagesEnabled::Never {
            return Err("Image display is disabled in the page configuration".to_string());
        }

        if let Some(picker) = self.picker.clone() {
            if picker.protocol_type() == ProtocolType::Halfblocks && !config.block_fallback {
                return Err(
                    "This terminal did not report Kitty, Sixel, or iTerm2 image support"
                        .to_string(),
                );
            }

            return Ok(picker);
        }

        if config.block_fallback {
            #[allow(deprecated)]
            let mut picker = Picker::from_fontsize((10, 20));
            picker.set_protocol_type(ProtocolType::Halfblocks);
            return Ok(picker);
        }

        Err(self
            .reason
            .clone()
            .unwrap_or_else(|| "No supported terminal image protocol was detected".to_string()))
    }

    fn unavailable_reason(&self, config: &PageImagesConfig) -> Option<String> {
        #[cfg(feature = "image-support")]
        {
            self.picker(config).err()
        }

        #[cfg(not(feature = "image-support"))]
        {
            let _ = config;
            Some(
                self.reason
                    .clone()
                    .unwrap_or_else(|| "wiki-tui was built without image support".to_string()),
            )
        }
    }
}

enum ImagePopupState {
    MetadataOnly(String),
    #[cfg(feature = "image-support")]
    Loading,
    #[cfg(feature = "image-support")]
    Loaded(StatefulProtocol),
    #[cfg(feature = "image-support")]
    Failed(String),
}

pub struct ImagePopupComponent {
    figure: FigureData,
    config: Arc<Config>,
    theme: Arc<Theme>,
    support: ImageSupport,
    state: ImagePopupState,

    #[cfg(feature = "image-support")]
    receiver: Option<UnboundedReceiver<Result<DynamicImage, String>>>,
}

impl ImagePopupComponent {
    pub fn new(
        figure: FigureData,
        config: Arc<Config>,
        theme: Arc<Theme>,
        support: ImageSupport,
    ) -> Self {
        let mut component = Self {
            figure,
            config,
            theme,
            support,
            state: ImagePopupState::MetadataOnly(String::new()),

            #[cfg(feature = "image-support")]
            receiver: None,
        };

        component.start_loading();
        component
    }

    fn start_loading(&mut self) {
        if self.figure.image.is_none() {
            self.state = ImagePopupState::MetadataOnly("No image URL was provided".to_string());
            return;
        }

        if let Some(reason) = self.support.unavailable_reason(&self.config.page.images) {
            self.state = ImagePopupState::MetadataOnly(reason);
            return;
        }

        #[cfg(feature = "image-support")]
        {
            let image = self.figure.image.as_ref().expect("image checked above");
            let url = image.url.clone();
            let (sender, receiver) = unbounded_channel();
            tokio::spawn(async move {
                let result = download_image(url).await;
                let _ = sender.send(result);
            });

            self.receiver = Some(receiver);
            self.state = ImagePopupState::Loading;
        }

        #[cfg(not(feature = "image-support"))]
        {
            self.state = ImagePopupState::MetadataOnly(
                "wiki-tui was built without image support".to_string(),
            );
        }
    }

    #[cfg(feature = "image-support")]
    fn poll_download(&mut self) {
        let Some(receiver) = self.receiver.as_mut() else {
            return;
        };

        let Ok(result) = receiver.try_recv() else {
            return;
        };

        self.receiver = None;
        match result {
            Ok(image) => match self.support.picker(&self.config.page.images) {
                Ok(picker) => {
                    self.state = ImagePopupState::Loaded(picker.new_resize_protocol(image));
                }
                Err(reason) => {
                    self.state = ImagePopupState::MetadataOnly(reason);
                }
            },
            Err(error) => self.state = ImagePopupState::Failed(error),
        }
    }

    fn metadata_lines(&self, status: Option<&str>) -> Vec<Line<'_>> {
        let mut lines = Vec::new();

        if let Some(caption) = self.figure.caption.as_deref() {
            lines.push(Line::from(vec!["Caption: ".bold(), caption.into()]));
        }

        if let Some(image) = self.figure.image.as_ref() {
            if let Some(alt) = image.alt.as_deref() {
                lines.push(Line::from(vec!["Alt: ".bold(), alt.into()]));
            }
            if let Some(title) = image.title.as_deref() {
                lines.push(Line::from(vec!["Title: ".bold(), title.into()]));
            }
            lines.push(Line::from(vec![
                "Source: ".bold(),
                image.url.as_str().into(),
            ]));
        }

        if let Some(status) = status {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(status.to_string()));
        }

        if lines.is_empty() {
            lines.push(Line::from("No image metadata available"));
        }

        lines
    }

    fn render_metadata(&self, f: &mut Frame<'_>, area: Rect, status: Option<&str>) {
        let paragraph = self
            .theme
            .default_paragraph(self.metadata_lines(status))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    }
}

#[cfg(feature = "image-support")]
async fn download_image(url: url::Url) -> Result<DynamicImage, String> {
    let response = reqwest::Client::new()
        .get(url.clone())
        .header(
            "User-Agent",
            format!(
                "wiki-tui/{} (https://github.com/Builditluc/wiki-tui)",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .send()
        .await
        .map_err(|error| format!("Failed to download image: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Failed to download image: {error}"))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("Failed to read image response: {error}"))?;

    image::load_from_memory(&bytes)
        .map_err(|error| format!("Failed to decode image from '{}': {error}", url.as_str()))
}

impl Component for ImagePopupComponent {
    fn handle_key_events(&mut self, key: KeyEvent) -> ActionResult {
        if self.config.bindings.global.pop_popup.matches_event(key)
            || matches!(key.code, KeyCode::Esc)
        {
            return Action::PopPopup.into();
        }

        ActionResult::Ignored
    }

    fn render(&mut self, f: &mut Frame<'_>, area: Rect) {
        #[cfg(feature = "image-support")]
        self.poll_download();

        let area = centered_rect(area, 80, 90);
        f.render_widget(Clear, area);
        f.render_widget(
            Block::default().style(Style::default().bg(self.theme.bg)),
            area,
        );

        let block = self
            .theme
            .default_block()
            .title("Image")
            .title_bottom(Line::from("<ESC> Close").right_aligned());
        let inner = block.inner(area);
        f.render_widget(block, area);

        #[cfg(feature = "image-support")]
        let metadata_len = self.metadata_lines(None).len() as u16;

        match &mut self.state {
            #[cfg(feature = "image-support")]
            ImagePopupState::Loading => {
                self.render_metadata(f, inner, Some("Loading image..."));
            }
            ImagePopupState::MetadataOnly(reason) => {
                let status = format!("{reason}. Showing image metadata instead.");
                self.render_metadata(f, inner, Some(&status));
            }
            #[cfg(feature = "image-support")]
            ImagePopupState::Failed(error) => {
                let status = format!("{error}. Showing image metadata instead.");
                self.render_metadata(f, inner, Some(&status));
            }
            #[cfg(feature = "image-support")]
            ImagePopupState::Loaded(protocol) => {
                let metadata_height = metadata_len.clamp(3, inner.height / 3);
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length(metadata_height.saturating_add(1)),
                    ])
                    .split(inner);

                f.render_stateful_widget(
                    StatefulImage::new().resize(Resize::Fit(None)),
                    chunks[0],
                    protocol,
                );
                self.render_metadata(f, chunks[1], None);
            }
        }
    }
}
