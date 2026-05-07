use ratatui::style::{Color, Modifier, Style};
use textwrap::wrap_algorithms::{wrap_optimal_fit, Penalties};
use tracing::warn;
use wiki_api::{
    document::{Data, Document, HeaderKind, Node, TableData, TableRowData, UnsupportedElement},
    page::Link,
};

use crate::renderer::Word;

use super::RenderedDocument;

const DISAMBIGUATION_PADDING: u8 = 1;
const DISAMBIGUATION_PREFIX: char = '|';

const BLOCKQUOTE_PADDING: u8 = 4;

const LIST_PADDING: u8 = 1;
const LIST_PREFIX: char = '-';

struct Renderer {
    rendered_lines: Vec<Vec<Word>>,
    links: Vec<(usize, usize)>,

    current_line: Vec<Word>,
    width: u16,

    text_style: Style,

    left_padding: u8,
    prefix: Option<char>,
}

impl<'a> Renderer {
    fn render_document(document: &'a Document, width: u16) -> RenderedDocument {
        if document.nodes.is_empty() {
            warn!("document contains no nodes, aborting the render");
            return RenderedDocument {
                lines: Vec::new(),
                links: Vec::new(),
            };
        }

        let mut renderer = Renderer {
            rendered_lines: Vec::new(),
            links: Vec::new(),

            current_line: Vec::new(),
            width,

            text_style: Style::default(),

            left_padding: 0,
            prefix: None,
        };

        renderer.render_node(document.nth(0).unwrap());

        RenderedDocument {
            lines: renderer.rendered_lines,
            links: renderer.links,
        }
    }

    /// Returns whether the last word of the current line is a whitespace
    fn is_last_whitespace(&self) -> bool {
        self.current_line
            .last()
            .map(|last| last.index == usize::MAX)
            .unwrap_or(false)
    }

    /// Returns whether the last rendered line is an empty one
    ///
    /// When the current line is not empty, this will return false
    fn is_last_empty(&self) -> bool {
        if !self.current_line.is_empty() {
            false
        } else {
            self.rendered_lines
                .last()
                .map(|last| last.is_empty())
                .unwrap_or(false)
        }
    }

    /// Adds a whitespace to the end of the current line
    ///
    /// The whitespace word has an index of `usize::MAX` and a width of `0` to not interfere with text wrapping. Note: If there already is a whitespace at the end of the current line, no whitespace will be added!
    fn add_whitespace(&mut self) {
        if self
            .current_line
            .last()
            .map(|word| word.index == usize::MAX)
            .unwrap_or(false)
        {
            return;
        }

        self.current_line.push(self.n_whitespace(1));
    }

    /// Returns a Word containing n amount of whitespace
    fn n_whitespace(&self, n: u8) -> Word {
        Word {
            index: usize::MAX,
            content: String::new(),
            style: Style::default(),
            width: 0.0,
            whitespace_width: n as f64,
            penalty_width: 0.0,
        }
    }

    /// Clears the current line
    ///
    /// When the current line is not empty already, it adds it to the rendered lines
    fn clear_line(&mut self) {
        if self.current_line.is_empty() {
            return;
        }

        self.rendered_lines
            .push(std::mem::take(&mut self.current_line));
    }

    /// Adds an empty line to the finished lines
    ///
    /// Clears the current line before adding the empty one
    fn add_empty_line(&mut self) {
        self.clear_line();
        self.rendered_lines.push(Vec::new());
    }

    fn current_width(&self) -> usize {
        let mut current_width: f64 = 0.0;
        for word in self.current_line.iter() {
            current_width = current_width + word.width + word.whitespace_width;
        }
        current_width as usize
    }

    /// Wraps and appends words
    ///
    /// This fills up the current line with words and wraps the remaining words into lines, appending them to the finished words. Note: This leaves the current line empty, except when there are not enough words to fill it up completely
    fn wrap_append(&mut self, words: Vec<Word>) {
        if words.is_empty() {
            return;
        }

        let current_width = self.current_width() as f64;
        let mut remaining_width = (self.width as f64) - current_width;

        // if the first word doesn't fit onto the current line, the line wrapping algorithm gets confuesed.
        // that means we have to clear it in this case
        if words.first().map(|word| word.width).unwrap_or_default() > remaining_width {
            remaining_width = self.width as f64;
            self.clear_line();
        }

        if self.current_line.is_empty() {
            remaining_width -= self.left_padding as f64;
            self.current_line.push(self.n_whitespace(self.left_padding));
            if let Some(prefix) = self.prefix {
                self.current_line.push(Word {
                    index: usize::MAX,
                    content: prefix.to_string(),
                    style: Style::default(),
                    width: 1.0,
                    whitespace_width: 1.0,
                    penalty_width: 0.0,
                });

                remaining_width -= 2.0; // subtract 2: 1 char & 1 whitespace
            }
        }

        let line_widths: [f64; 2] = [remaining_width, self.width as f64];
        let mut wrapped_lines: Vec<Vec<Word>> =
            wrap_optimal_fit(&words, &line_widths, &Penalties::default())
                .unwrap()
                .into_iter()
                .map(|word| word.to_vec())
                .collect();

        self.current_line.append(&mut wrapped_lines.remove(0));

        // add prefixes
        if let Some(prefix) = self.prefix {
            for line in wrapped_lines.iter_mut() {
                line.insert(
                    0,
                    Word {
                        index: usize::MAX,
                        content: prefix.to_string(),
                        style: Style::default(),
                        width: 1.0,
                        whitespace_width: 1.0,
                        penalty_width: 0.0,
                    },
                );
            }
        }

        // indent the current line
        for line in wrapped_lines.iter_mut() {
            line.insert(0, self.n_whitespace(self.left_padding));
        }

        if let Some(last_line) = wrapped_lines.pop() {
            self.clear_line();
            self.current_line = last_line;
            self.rendered_lines.append(&mut wrapped_lines)
        }
    }

    /// Adds an empty line only if the last line is not empty
    fn ensure_empty_line(&mut self) {
        if !self.is_last_empty() {
            self.add_empty_line();
        }
    }

    /// Adds a modifier to the current text style
    fn add_modifier(&mut self, modifier: Modifier) {
        self.text_style = self.text_style.add_modifier(modifier);
    }

    /// Removes a modifier from the current text style
    fn remove_modifier(&mut self, modifier: Modifier) {
        self.text_style = self.text_style.remove_modifier(modifier);
    }

    /// Changes the foreground color of the text style
    fn set_text_fg(&mut self, color: Color) {
        self.text_style = self.text_style.fg(color);
    }

    /// Resets the foreground color of the text style
    fn reset_text_fg(&mut self) {
        self.text_style.fg = None;
    }

    /// Adds n spaces to the left padding
    fn add_n_padding(&mut self, n: u8) {
        self.left_padding = self.left_padding.saturating_add(n);
    }

    /// Removes n spaces from the left padding
    fn remove_n_padding(&mut self, n: u8) {
        self.left_padding = self.left_padding.saturating_sub(n);
    }

    /// Sets the prefix to a given value
    fn set_prefix(&mut self, prefix: char) {
        self.prefix = Some(prefix);
    }

    /// Resets the prefix
    fn reset_prefix(&mut self) {
        self.prefix = None;
    }

    fn add_horizontal_line(&mut self) {
        let remaining_width = (self.width as usize) - self.current_width();
        let line = Word {
            index: usize::MAX,
            content: "─".repeat(remaining_width),
            style: self.text_style,
            width: remaining_width as f64,
            whitespace_width: 0.0,
            penalty_width: 0.0,
        };
        self.current_line.push(line);
        self.clear_line();
    }

    fn render_children(&mut self, node: Node<'a>) {
        for child in node.children() {
            self.render_node(child);
        }
    }

    fn render_section(&mut self, node: Node<'a>) {
        if !matches!(node.data(), Data::Section { .. }) {
            warn!("expected section data, got other data");
            return;
        }

        self.ensure_empty_line();

        self.render_children(node);

        self.ensure_empty_line();
    }

    fn render_header(&mut self, node: Node<'a>) {
        let Data::Header { kind, .. } = node.data() else {
            warn!("expected header data, got other data");
            return;
        };

        self.ensure_empty_line();

        if !matches!(kind, &HeaderKind::Main | &HeaderKind::Sub) {
            self.add_modifier(Modifier::BOLD);
        }
        self.set_text_fg(Color::Red);

        self.render_children(node);

        if !matches!(kind, &HeaderKind::Main | &HeaderKind::Sub) {
            self.remove_modifier(Modifier::BOLD);
        }
        self.reset_text_fg();

        if matches!(kind, &HeaderKind::Main | &HeaderKind::Sub) {
            self.clear_line();
            self.add_horizontal_line();
        }

        self.ensure_empty_line();
    }

    fn render_text(&mut self, node: Node<'a>) {
        let contents = match node.data() {
            Data::Text { contents } => contents,
            _ => {
                warn!("expected text data, got other data");
                return;
            }
        };

        self.render_string(contents, node.index());
        self.render_children(node);
    }

    fn render_string(&mut self, content: &str, index: usize) {
        const TEXT_SPECIAL_CHARACTERS: [char; 9] = [',', '.', ':', ';', '\"', '\'', '!', '@', '%'];
        if content.starts_with(TEXT_SPECIAL_CHARACTERS) && self.is_last_whitespace() {
            self.current_line.pop();
        }

        let has_trailing_whitespace = content.ends_with(' ');
        let mut words: Vec<Word> = content
            .split_whitespace()
            .map(|word| Word {
                index,
                content: word.to_string(),
                style: self.text_style,
                width: word.chars().count() as f64,
                whitespace_width: 1.0,
                penalty_width: 0.0,
            })
            .collect();

        if !has_trailing_whitespace {
            if let Some(word) = words.last_mut() {
                word.whitespace_width = 0.0;
            }
        }

        self.wrap_append(words);
    }

    fn render_block_element(&mut self, node: Node<'a>) {
        self.ensure_empty_line();
        self.render_children(node);
        self.ensure_empty_line();
    }

    fn render_span(&mut self, node: Node<'a>) {
        self.render_children(node);
        self.add_whitespace();
    }

    fn render_reflink(&mut self, node: Node<'a>) {
        self.add_modifier(Modifier::ITALIC);
        self.set_text_fg(Color::Gray);

        self.render_children(node);

        self.reset_text_fg();
        self.remove_modifier(Modifier::ITALIC);

        self.add_whitespace();
    }

    fn render_disambiguation(&mut self, node: Node<'a>) {
        self.ensure_empty_line();

        self.add_modifier(Modifier::ITALIC);
        self.add_n_padding(DISAMBIGUATION_PADDING);
        self.set_prefix(DISAMBIGUATION_PREFIX);

        self.render_children(node);

        self.reset_prefix();
        self.remove_n_padding(DISAMBIGUATION_PADDING);
        self.remove_modifier(Modifier::ITALIC);

        self.ensure_empty_line();
    }

    fn render_block_quote(&mut self, node: Node<'a>) {
        self.add_n_padding(BLOCKQUOTE_PADDING);

        self.render_block_element(node);

        self.remove_n_padding(BLOCKQUOTE_PADDING);
    }

    fn render_list(&mut self, node: Node<'a>) {
        self.ensure_empty_line();

        self.add_n_padding(LIST_PADDING);

        self.render_children(node);

        self.remove_n_padding(LIST_PADDING);

        self.ensure_empty_line();
    }

    fn render_list_item(&mut self, node: Node<'a>) {
        self.clear_line();
        self.current_line.push(Word {
            index: usize::MAX,
            content: format!("{}{LIST_PREFIX}", " ".repeat(self.left_padding as usize)),
            style: Style::default(),
            width: 1.0,
            whitespace_width: 1.0,
            penalty_width: 0.0,
        });
        self.add_n_padding(2);

        self.render_children(node);

        self.remove_n_padding(2);
        self.clear_line();
    }

    fn render_description_list_term(&mut self, node: Node<'a>) {
        self.clear_line();
        self.render_children(node);
        self.clear_line();
    }

    fn render_description_list_description(&mut self, node: Node<'a>) {
        self.clear_line();
        self.render_children(node);
        self.clear_line();
    }

    fn render_bold(&mut self, node: Node<'a>) {
        self.add_modifier(Modifier::BOLD);

        self.render_children(node);

        self.remove_modifier(Modifier::BOLD);
        self.add_whitespace();
    }

    fn render_italic(&mut self, node: Node<'a>) {
        self.add_modifier(Modifier::ITALIC);
        self.set_text_fg(Color::Blue);

        self.render_children(node);

        self.reset_text_fg();
        self.remove_modifier(Modifier::ITALIC);
        self.add_whitespace();
    }

    fn render_linebreak(&mut self, node: Node<'a>) {
        self.clear_line();
        self.render_children(node);
    }

    fn render_link(&mut self, node: Node<'a>, link: Link) {
        self.links.push((self.rendered_lines.len(), node.index()));

        match link {
            Link::Internal(_) => self.render_wiki_link(node),
            Link::Anchor(_) => self.render_wiki_link(node),
            Link::RedLink(_) => self.render_red_link(node),
            Link::MediaLink(_) => self.render_media_link(node),
            Link::External(_) => self.render_external_link(node),
            Link::ExternalToInternal(_) => self.render_external_link(node),
        }
    }

    fn render_wiki_link(&mut self, node: Node<'a>) {
        self.set_text_fg(Color::Blue);
        self.render_children(node);
        self.reset_text_fg();

        self.add_whitespace();
    }

    fn render_red_link(&mut self, node: Node<'a>) {
        self.add_modifier(Modifier::ITALIC);
        self.set_text_fg(Color::Red);

        self.render_children(node);

        self.reset_text_fg();
        self.remove_modifier(Modifier::ITALIC);
        self.add_whitespace();
    }

    fn render_media_link(&mut self, node: Node<'a>) {
        self.add_modifier(Modifier::ITALIC);
        self.set_text_fg(Color::Blue);

        self.render_children(node);

        self.reset_text_fg();
        self.remove_modifier(Modifier::ITALIC);
        self.add_whitespace();
    }

    fn render_external_link(&mut self, node: Node<'a>) {
        self.add_modifier(Modifier::ITALIC);

        self.render_children(node);

        self.remove_modifier(Modifier::ITALIC);
        self.add_whitespace();
    }

    fn render_table(&mut self, node: Node<'a>, table: &TableData) {
        self.ensure_empty_line();

        if let Some(caption) = table.caption.as_deref() {
            self.add_modifier(Modifier::ITALIC);
            self.set_text_fg(Color::Gray);
            self.render_string(&format!("[Table] {caption}"), node.index());
            self.clear_line();
            self.reset_text_fg();
            self.remove_modifier(Modifier::ITALIC);
        }

        let widths = Self::table_column_widths(table, self.width as usize);
        if widths.is_empty() {
            self.add_empty_line();
            return;
        }

        let separator = Self::format_table_separator(&widths);
        for row in table.rows.iter() {
            let row_style = if Self::is_header_row(row) {
                self.text_style.add_modifier(Modifier::BOLD)
            } else {
                self.text_style
            };

            self.push_rendered_line(
                Self::format_table_row(row, &widths),
                node.index(),
                row_style,
            );

            if Self::is_header_row(row) {
                self.push_rendered_line(separator.clone(), node.index(), self.text_style);
            }
        }

        self.add_empty_line();
    }

    fn table_column_widths(table: &TableData, available_width: usize) -> Vec<usize> {
        let column_count = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or_default();

        if column_count == 0 {
            return Vec::new();
        }

        let max_cell_width = Self::max_table_cell_width(available_width, column_count);
        let mut widths = vec![1; column_count];

        for row in table.rows.iter() {
            for (index, cell) in row.cells.iter().enumerate() {
                let cell_width = cell.text.chars().count().max(1).min(max_cell_width);
                widths[index] = widths[index].max(cell_width);
            }
        }

        widths
    }

    fn max_table_cell_width(available_width: usize, column_count: usize) -> usize {
        let separator_width = column_count.saturating_mul(3).saturating_add(1);

        available_width
            .saturating_sub(separator_width)
            .checked_div(column_count)
            .unwrap_or(1)
            .max(1)
    }

    fn is_header_row(row: &TableRowData) -> bool {
        row.cells.iter().any(|cell| cell.header)
    }

    fn format_table_row(row: &TableRowData, widths: &[usize]) -> String {
        let mut line = String::from("|");

        for (index, width) in widths.iter().enumerate() {
            let text = row
                .cells
                .get(index)
                .map(|cell| cell.text.as_str())
                .unwrap_or("");
            line.push(' ');
            line.push_str(&Self::fit_table_cell(text, *width));
            line.push(' ');
            line.push('|');
        }

        line
    }

    fn format_table_separator(widths: &[usize]) -> String {
        let mut line = String::from("|");

        for width in widths {
            line.push(' ');
            line.push_str(&"-".repeat(*width));
            line.push(' ');
            line.push('|');
        }

        line
    }

    fn fit_table_cell(text: &str, width: usize) -> String {
        let mut value: String = text.chars().take(width).collect();
        let value_width = value.chars().count();

        if value_width < width {
            value.push_str(&" ".repeat(width - value_width));
        }

        value
    }

    fn push_rendered_line(&mut self, line: String, index: usize, style: Style) {
        let content = format!("{}{}", " ".repeat(self.left_padding as usize), line);
        let width = content.chars().count() as f64;

        self.current_line.push(Word {
            index,
            content,
            style,
            width,
            whitespace_width: 0.0,
            penalty_width: 0.0,
        });
        self.clear_line();
    }

    fn render_unsupported_element(
        &mut self,
        inline: bool,
        element: &UnsupportedElement,
        index: usize,
    ) {
        if inline {
            self.add_modifier(Modifier::ITALIC);

            self.add_whitespace();

            self.set_text_fg(Color::DarkGray);
            self.render_string("[x]", index);
            self.reset_text_fg();

            self.add_whitespace();

            self.remove_modifier(Modifier::ITALIC);

            return;
        }

        self.ensure_empty_line();
        self.add_modifier(Modifier::ITALIC);

        let message = match element {
            UnsupportedElement::Table => "<Unsupported Element 'Table'>",
            UnsupportedElement::Image => "<Unsupported Element 'Image'>",
            UnsupportedElement::Figure => "<Unsupported Element 'Figure'>",
            UnsupportedElement::MathElement => "<Unsupported Element 'Math Element'>",
            UnsupportedElement::PreformattedText => "<Unsupported Element 'PreformattedText'>",
        };

        self.render_string(message, index);

        self.remove_modifier(Modifier::ITALIC);
        self.add_empty_line();
    }

    fn render_node(&mut self, node: Node<'a>) {
        match node.data() {
            Data::Section { id: _ } => self.render_section(node),
            Data::Header { id: _, kind: _ } => self.render_header(node),
            Data::Text { contents: _ } => self.render_text(node),
            Data::Division => self.render_block_element(node),
            Data::Paragraph => self.render_block_element(node),
            Data::Span => self.render_span(node),
            Data::Reflink => self.render_reflink(node),
            Data::Hatnote => self.render_block_element(node),
            Data::RedirectMessage => self.render_block_element(node),
            Data::Disambiguation => self.render_disambiguation(node),
            Data::Blockquote => self.render_block_quote(node),
            Data::OrderedList => self.render_list(node),
            Data::UnorderedList => self.render_list(node),
            Data::ListItem => self.render_list_item(node),
            Data::DescriptionList => self.render_block_element(node),
            Data::DescriptionListTerm => self.render_description_list_term(node),
            Data::DerscriptionListDescription => self.render_description_list_description(node),
            Data::Bold => self.render_bold(node),
            Data::Italic => self.render_italic(node),
            Data::Linebreak => self.render_linebreak(node),
            Data::Link(link) => self.render_link(node, link.clone()),
            Data::Table(table) => self.render_table(node, table),
            Data::Unknown => self.render_children(node),
            Data::Unsupported(element) => {
                self.render_unsupported_element(false, element, node.index())
            }
            Data::UnsupportedInline(element) => {
                self.render_unsupported_element(true, element, node.index())
            }
        }
    }
}

pub fn render_document(document: &Document, width: u16) -> RenderedDocument {
    Renderer::render_document(document, width)
}

#[cfg(test)]
mod tests {
    use wiki_api::document::{Data, Document, Raw, TableCellData, TableData, TableRowData};

    use super::render_document;

    fn document_with(data: Data) -> Document {
        Document {
            nodes: vec![Raw {
                index: 0,
                parent: None,
                prev: None,
                next: None,
                first_child: None,
                last_child: None,
                data,
            }],
        }
    }

    fn cell(header: bool, text: &str) -> TableCellData {
        TableCellData {
            header,
            text: text.to_string(),
        }
    }

    fn row(cells: Vec<TableCellData>) -> TableRowData {
        TableRowData { cells }
    }

    fn table() -> TableData {
        TableData {
            caption: Some("Planets".to_string()),
            rows: vec![
                row(vec![cell(true, "Name"), cell(true, "Moons")]),
                row(vec![cell(false, "Earth"), cell(false, "1")]),
            ],
        }
    }

    fn rendered_lines(document: &Document, width: u16) -> Vec<String> {
        render_document(document, width)
            .lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|word| {
                        format!(
                            "{}{}",
                            word.content,
                            " ".repeat(word.whitespace_width as usize)
                        )
                    })
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn renders_table_instead_of_unsupported_element() {
        let document = document_with(Data::Table(table()));
        let text = rendered_lines(&document, 120).join("\n");

        assert!(text.contains("[Table] Planets"));
        assert!(text.contains("| Name"));
        assert!(text.contains("| Earth"));
        assert!(!text.contains("Unsupported Element"));
    }

    #[test]
    fn renders_header_separator() {
        let document = document_with(Data::Table(table()));
        let text = rendered_lines(&document, 120).join("\n");

        assert!(text.contains("| ----- | ----- |"));
    }

    #[test]
    fn renders_missing_cells_as_empty_columns() {
        let document = document_with(Data::Table(TableData {
            caption: None,
            rows: vec![
                row(vec![cell(true, "Name"), cell(true, "Moons")]),
                row(vec![cell(false, "Mars")]),
            ],
        }));
        let text = rendered_lines(&document, 120).join("\n");

        assert!(text.contains("| Mars |       |"));
    }

    #[test]
    fn renders_narrow_table_without_panicking() {
        let document = document_with(Data::Table(TableData {
            caption: Some("Very narrow".to_string()),
            rows: vec![row(vec![
                cell(true, "Long header"),
                cell(true, "Another long header"),
                cell(true, "Third long header"),
            ])],
        }));
        let lines = rendered_lines(&document, 8);

        assert!(lines.iter().any(|line| line.contains("[Table]")));
        assert!(lines.iter().any(|line| line.starts_with("| ")));
    }
}
