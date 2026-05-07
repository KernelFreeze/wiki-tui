use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use std::str::FromStr;
use tracing::{trace, warn};
use url::Url;

use crate::{
    document::{
        Data, FigureData, HeaderKind, ImageData, Raw, TableCellData, TableData, TableRowData,
        UnsupportedElement,
    },
    languages::Language,
    page::{
        link_data::{AnchorData, ExternalData, ExternalToInteralData, InternalData, MediaData},
        Link,
    },
    search::Namespace,
    Endpoint,
};

// TODO: remove Parser and replace it with normal functions and helper functions
pub trait Parser {
    fn parse_document(document: &str, endpoint: Endpoint, language: Language) -> Self;
    fn nodes(self) -> Vec<Raw>;
}

pub struct WikipediaParser {
    nodes: Vec<Raw>,
    endpoint: Endpoint,
    language: Language,
}

impl WikipediaParser {
    fn parse_node(
        &mut self,
        node: &Handle,
        parent: Option<usize>,
        prev: Option<usize>,
    ) -> Option<usize> {
        match node.data {
            NodeData::Document => {
                let mut prev = None;
                for child in node.children.borrow().iter() {
                    prev = self.parse_node(child, parent, prev)
                }
                None
            }
            NodeData::Text { ref contents } => {
                let data = Data::Text {
                    contents: contents.borrow().to_string(),
                };
                Some(self.push_node(data, parent, prev))
            }
            NodeData::Element {
                ref name,
                ref attrs,
                ..
            } => {
                let name = name.local.to_string();
                let attrs: Vec<(String, String)> = attrs
                    .borrow()
                    .iter()
                    .map(|attr| (attr.name.local.to_string(), attr.value.to_string()))
                    .collect();

                let mut ignore_children = false;

                let data = match name.as_str() {
                    "head" | "style" | "link" => return prev,

                    "table" => {
                        ignore_children = true;
                        Data::Table(Self::parse_table(node))
                    }
                    "img" | "image" => {
                        ignore_children = true;
                        match self.parse_image(&attrs) {
                            Some(image) => Data::Image(image),
                            None => Data::Unsupported(UnsupportedElement::Image),
                        }
                    }
                    "figure" => {
                        ignore_children = true;
                        match self.parse_figure(node) {
                            Some(figure) => Data::Figure(figure),
                            None => Data::Unsupported(UnsupportedElement::Figure),
                        }
                    }
                    "pre" => {
                        ignore_children = true;
                        Data::Unsupported(UnsupportedElement::PreformattedText)
                    }

                    "span"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class"
                                && (value.contains("texhtml") || value.contains("mwe-math-element"))
                        }) =>
                    {
                        ignore_children = true;
                        Data::UnsupportedInline(UnsupportedElement::MathElement)
                    }

                    "ul" if attrs.iter().any(|(name, value)| {
                        name.as_str() == "class" && value.contains("portalbox")
                    }) =>
                    {
                        trace!("ignoring 'ul' class: 'portalbox'");
                        return prev;
                    }

                    "div"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class"
                                && (value.contains("toc") || value.contains("quotebox"))
                        }) =>
                    {
                        trace!("ignoring 'div': class: 'toc' || 'quotebox'");
                        return prev;
                    }

                    "div"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class" && value.contains("mw-empty-elt")
                        }) =>
                    {
                        trace!("ignoring 'div': class: 'mw-empty-elt'");
                        return prev;
                    }

                    "span"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class" && value.contains("cs1-maint")
                        }) =>
                    {
                        trace!("ignoring 'span': class: 'cs1-maint'");
                        return prev;
                    }

                    _ if attrs.iter().any(|(name, value)| {
                        name.as_str() == "class" && value.contains("noprint")
                    }) =>
                    {
                        trace!("ignoring '{name}': class: 'noprint'");
                        return prev;
                    }

                    "span"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class" && value.contains("mw-editsection")
                        }) =>
                    {
                        trace!("ignoring 'span': class: 'mw-editsection'");
                        return prev;
                    }

                    "span"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "typeof" && value.contains("mw:Nowiki")
                        }) =>
                    {
                        trace!("ignoring 'span': class: 'mw:Nowiki'");
                        return prev;
                    }

                    "span"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class" && value.contains("mw-reflink-text")
                        }) =>
                    {
                        Data::Reflink
                    }

                    "section" => self.parse_section(attrs.iter()).unwrap_or_default(),
                    "h1" => self
                        .parse_header(attrs.iter(), HeaderKind::Main)
                        .unwrap_or_default(),

                    "h2" => self
                        .parse_header(attrs.iter(), HeaderKind::Sub)
                        .unwrap_or_default(),
                    "h3" => self
                        .parse_header(attrs.iter(), HeaderKind::Section)
                        .unwrap_or_default(),
                    "h4" => self
                        .parse_header(attrs.iter(), HeaderKind::Subsection)
                        .unwrap_or_default(),
                    "h5" => self
                        .parse_header(attrs.iter(), HeaderKind::Minor)
                        .unwrap_or_default(),
                    "h6" => self
                        .parse_header(attrs.iter(), HeaderKind::Detail)
                        .unwrap_or_default(),

                    "blockquote" => Data::Blockquote,

                    "ol" => Data::OrderedList,
                    "ul" => Data::UnorderedList,
                    "li" => Data::ListItem,

                    "dl" => Data::DescriptionList,
                    "dt" => Data::DescriptionListTerm,
                    "dd" => Data::DerscriptionListDescription,

                    "br" => Data::Linebreak,

                    "b" => Data::Bold,
                    "i" => Data::Italic,

                    "p" => Data::Paragraph,
                    "span" => Data::Span,

                    "div"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class" && value.contains("redirectMsg")
                        }) =>
                    {
                        Data::RedirectMessage
                    }

                    "div"
                        if attrs.iter().any(|(name, value)| {
                            name.as_str() == "class" && value.contains("hatnote")
                        }) =>
                    {
                        Data::Disambiguation
                    }

                    "a" => {
                        Self::parse_link(&self.endpoint, self.language, &attrs).unwrap_or_default()
                    }

                    "div" => Data::Division,
                    _ => {
                        warn!("unknown node '{name}'");
                        Data::Unknown
                    }
                };

                let index = self.push_node(data, parent, prev);

                if ignore_children {
                    return Some(index);
                }

                let mut prev = None;
                for child in node.children.borrow().iter() {
                    prev = self.parse_node(child, Some(index), prev)
                }
                Some(index)
            }
            NodeData::ProcessingInstruction { .. }
            | NodeData::Doctype { .. }
            | NodeData::Comment { .. } => prev,
        }
    }

    fn push_node(&mut self, data: Data, parent: Option<usize>, prev: Option<usize>) -> usize {
        let index = self.nodes.len();

        self.nodes.push(Raw {
            index,
            parent,
            prev,
            next: None,
            first_child: None,
            last_child: None,
            data,
        });

        if let Some(parent) = parent {
            let parent = &mut self.nodes[parent];
            if parent.first_child.is_none() {
                parent.first_child = Some(index);
            }
            parent.last_child = Some(index);
        }

        if let Some(prev) = prev {
            self.nodes[prev].next = Some(index);
        }

        index
    }

    fn parse_image(&self, attrs: &[(String, String)]) -> Option<ImageData> {
        let src = Self::attr(attrs, "src")
            .or_else(|| Self::attr(attrs, "href"))
            .or_else(|| Self::attr(attrs, "srcset").and_then(Self::first_srcset_url))?;

        Some(ImageData {
            url: self.resolve_url(src)?,
            alt: Self::non_empty_attr(attrs, "alt"),
            title: Self::non_empty_attr(attrs, "title"),
        })
    }

    fn parse_figure(&self, node: &Handle) -> Option<FigureData> {
        let image = self.find_first_image(node);
        let caption = Self::find_figcaption_text(node);

        if image.is_none() && caption.is_none() {
            return None;
        }

        Some(FigureData { image, caption })
    }

    fn find_first_image(&self, node: &Handle) -> Option<ImageData> {
        if let Some(attrs) = Self::element_attrs(node, &["img", "image"]) {
            if let Some(image) = self.parse_image(&attrs) {
                return Some(image);
            }
        }

        for child in node.children.borrow().iter() {
            if let Some(image) = self.find_first_image(child) {
                return Some(image);
            }
        }

        None
    }

    fn find_figcaption_text(node: &Handle) -> Option<String> {
        if Self::element_attrs(node, &["figcaption"]).is_some() {
            return Self::node_text(node);
        }

        for child in node.children.borrow().iter() {
            if let Some(caption) = Self::find_figcaption_text(child) {
                return Some(caption);
            }
        }

        None
    }

    fn element_attrs(node: &Handle, names: &[&str]) -> Option<Vec<(String, String)>> {
        let NodeData::Element {
            ref name,
            ref attrs,
            ..
        } = node.data
        else {
            return None;
        };

        let name = name.local.to_string();
        if !names.iter().any(|candidate| *candidate == name) {
            return None;
        }

        Some(
            attrs
                .borrow()
                .iter()
                .map(|attr| (attr.name.local.to_string(), attr.value.to_string()))
                .collect(),
        )
    }

    fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
        attrs
            .iter()
            .find(|(attr_name, _)| attr_name == name)
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
    }

    fn non_empty_attr(attrs: &[(String, String)], name: &str) -> Option<String> {
        Self::attr(attrs, name).map(ToOwned::to_owned)
    }

    fn first_srcset_url(srcset: &str) -> Option<&str> {
        srcset
            .split(',')
            .find_map(|candidate| candidate.split_whitespace().next())
    }

    fn resolve_url(&self, value: &str) -> Option<Url> {
        let value = if value.starts_with("//") {
            format!("{}:{value}", self.endpoint.scheme())
        } else {
            value.to_string()
        };

        self.endpoint.join(&value).ok()
    }

    fn parse_table(node: &Handle) -> TableData {
        let mut caption = None;
        let mut rows = Vec::new();

        Self::collect_table_parts(node, true, &mut caption, &mut rows);

        TableData { caption, rows }
    }

    fn collect_table_parts(
        node: &Handle,
        is_root: bool,
        caption: &mut Option<String>,
        rows: &mut Vec<TableRowData>,
    ) {
        for child in node.children.borrow().iter() {
            match Self::element_name(child).as_deref() {
                Some("table") if !is_root => {}
                Some("caption") => {
                    if caption.is_none() {
                        *caption = Self::node_text(child);
                    }
                }
                Some("tr") => {
                    if let Some(row) = Self::parse_table_row(child) {
                        rows.push(row);
                    }
                }
                Some("thead" | "tbody" | "tfoot") => {
                    Self::collect_table_parts(child, false, caption, rows);
                }
                _ => {
                    Self::collect_table_parts(child, false, caption, rows);
                }
            }
        }
    }

    fn parse_table_row(node: &Handle) -> Option<TableRowData> {
        let cells: Vec<TableCellData> = node
            .children
            .borrow()
            .iter()
            .filter_map(|child| match Self::element_name(child).as_deref() {
                Some("th") => Some(TableCellData {
                    header: true,
                    text: Self::node_text(child).unwrap_or_default(),
                }),
                Some("td") => Some(TableCellData {
                    header: false,
                    text: Self::node_text(child).unwrap_or_default(),
                }),
                _ => None,
            })
            .collect();

        if cells.is_empty() || cells.iter().all(|cell| cell.text.is_empty()) {
            return None;
        }

        Some(TableRowData { cells })
    }

    fn element_name(node: &Handle) -> Option<String> {
        let NodeData::Element { ref name, .. } = node.data else {
            return None;
        };

        Some(name.local.to_string())
    }

    fn node_text(node: &Handle) -> Option<String> {
        let mut text = String::new();
        Self::collect_node_text(node, &mut text);
        Self::normalize_text(&text)
    }

    fn collect_node_text(node: &Handle, text: &mut String) {
        match node.data {
            NodeData::Text { ref contents } => {
                text.push_str(&contents.borrow());
                text.push(' ');
            }
            NodeData::Element { ref name, .. } => match name.local.as_ref() {
                "br" => text.push(' '),
                "style" | "script" | "table" => {}
                _ => {
                    for child in node.children.borrow().iter() {
                        Self::collect_node_text(child, text);
                    }
                }
            },
            _ => {}
        }
    }

    fn normalize_text(value: &str) -> Option<String> {
        let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }

    fn parse_section<'a>(
        &mut self,
        mut attrs: impl Iterator<Item = &'a (String, String)>,
    ) -> Option<Data> {
        let section_id = attrs
            .find(|(name, _)| name.as_str() == "data-mw-section-id")
            .map(|(_, value)| value)?;
        let section_id = usize::from_str(section_id)
            .map_err(|err| warn!("section-id not a usize, '{err:?}'"))
            .ok()?;

        Some(Data::Section { id: section_id })
    }

    fn parse_header<'a>(
        &mut self,
        mut attrs: impl Iterator<Item = &'a (String, String)>,
        kind: HeaderKind,
    ) -> Option<Data> {
        let header_id = attrs
            .find(|(name, _)| name.as_str() == "id")
            .map(|(_, value)| value.to_owned())?;

        Some(Data::Header {
            id: header_id,
            kind,
        })
    }

    fn parse_link(endpoint: &Url, language: Language, attrs: &[(String, String)]) -> Option<Data> {
        let href = attrs
            .iter()
            .find(|(name, _)| name.as_str() == "href")
            .map(|(_, value)| value.to_owned())?;

        let title = attrs
            .iter()
            .find(|(name, _)| name.as_str() == "title")
            .map(|(_, value)| value.to_owned())
            .unwrap_or_default();

        let link_url = endpoint.join(&href).ok()?;
        let link_type: &str = match attrs
            .iter()
            .find(|(name, _)| name.as_str() == "rel")
            .map(|(_, value)| value.to_owned())?
            .as_str()
        {
            "mw:WikiLink" => "wiki",
            "mw:MediaLink" => "media",
            "mw:ExtLink" => "external",
            _ => "",
        };

        let anchor = link_url.fragment().map(|fragment| AnchorData {
            title: title.to_string(),
            anchor: fragment.to_string(),
        });

        if link_type == "wiki" {
            let namespace = Namespace::Main;

            let is_same_wiki = link_url.domain() == endpoint.domain();
            if !is_same_wiki {
                return Some(Data::Link(Link::ExternalToInternal(
                    ExternalToInteralData {},
                )));
            }

            let page = link_url.path_segments()?.next_back()?;

            const NAMESPACE_DELIMITER: char = ':';
            let (namespace, page) =
                if let Some((ns_str, page_str)) = page.split_once(NAMESPACE_DELIMITER) {
                    (
                        Namespace::from_string(ns_str).unwrap_or_else(|| {
                            warn!("invalid namespace '{}', using default", ns_str);
                            namespace
                        }),
                        page_str,
                    )
                } else {
                    (namespace, page)
                };

            // we get the language from the host
            // for wikipedia, the host looks like this
            //      [lang].wikipedia.org/
            // where [lang] is the language code, for example
            //      en.wikipedia.org/
            // for the english wikipedia

            let lang_str = link_url
                .host_str()
                .and_then(|x| x.split_once('.').map(|x| x.0));

            let language = match lang_str {
                Some(str) => Language::from_str(str).unwrap_or(language),
                None => language,
            };

            let link_data = InternalData {
                namespace,
                page: page.to_string(),
                title,
                endpoint: endpoint.clone(),
                language,
                anchor,
            };

            return Some(Data::Link(Link::Internal(link_data)));
        }

        if link_type == "media" {
            return Some(Data::Link(Link::MediaLink(MediaData {
                url: link_url,
                title,
            })));
        }

        if link_type == "external" {
            return Some(Data::Link(Link::External(ExternalData { url: link_url })));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Parser, WikipediaParser};
    use crate::{
        document::{Data, UnsupportedElement},
        languages::Language,
        Endpoint,
    };

    fn parse_nodes(html: &str) -> Vec<crate::document::Raw> {
        WikipediaParser::parse_document(
            html,
            Endpoint::parse("https://en.wikipedia.org/w/api.php").unwrap(),
            Language::default(),
        )
        .nodes()
    }

    #[test]
    fn parses_image_with_alt_title_and_src() {
        let nodes = parse_nodes(
            r#"<section><img src="//upload.wikimedia.org/image.png" alt="Rocket" title="Rocket title"></section>"#,
        );

        let image = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Image(image) => Some(image),
                _ => None,
            })
            .expect("image node");

        assert_eq!(image.url.as_str(), "https://upload.wikimedia.org/image.png");
        assert_eq!(image.alt.as_deref(), Some("Rocket"));
        assert_eq!(image.title.as_deref(), Some("Rocket title"));
    }

    #[test]
    fn parses_figure_caption_and_image() {
        let nodes = parse_nodes(
            r#"<section><figure><img src="/wiki/Special:FilePath/Rocket.jpg" alt="A rocket"><figcaption>Rocket launch caption</figcaption></figure></section>"#,
        );

        let figure = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Figure(figure) => Some(figure),
                _ => None,
            })
            .expect("figure node");

        assert_eq!(figure.caption.as_deref(), Some("Rocket launch caption"));
        assert_eq!(
            figure.image.as_ref().map(|image| image.alt.as_deref()),
            Some(Some("A rocket"))
        );
    }

    #[test]
    fn parses_caption_only_figure() {
        let nodes = parse_nodes(
            "<section><figure><figcaption>Only caption</figcaption></figure></section>",
        );

        let figure = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Figure(figure) => Some(figure),
                _ => None,
            })
            .expect("figure node");

        assert!(figure.image.is_none());
        assert_eq!(figure.caption.as_deref(), Some("Only caption"));
    }

    #[test]
    fn parses_first_srcset_url() {
        let nodes = parse_nodes(
            r#"<section><img srcset="//upload.wikimedia.org/a.png 1x, //upload.wikimedia.org/b.png 2x" alt="Rocket"></section>"#,
        );

        let image = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Image(image) => Some(image),
                _ => None,
            })
            .expect("image node");

        assert_eq!(image.url.as_str(), "https://upload.wikimedia.org/a.png");
    }

    #[test]
    fn parses_table_caption_header_and_body_rows() {
        let nodes = parse_nodes(
            r#"
            <section>
                <table class="wikitable">
                    <caption>Planets</caption>
                    <tr><th>Name</th><th>Moons</th></tr>
                    <tr><td>Earth</td><td>1</td></tr>
                </table>
            </section>
            "#,
        );

        let table = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Table(table) => Some(table),
                _ => None,
            })
            .expect("table node");

        assert_eq!(table.caption.as_deref(), Some("Planets"));
        assert_eq!(table.rows.len(), 2);
        assert!(table.rows[0].cells.iter().all(|cell| cell.header));
        assert_eq!(table.rows[0].cells[0].text, "Name");
        assert_eq!(table.rows[1].cells[0].text, "Earth");
    }

    #[test]
    fn parses_table_sections() {
        let nodes = parse_nodes(
            r#"
            <table>
                <thead><tr><th>Year</th><th>Event</th></tr></thead>
                <tbody><tr><td>1969</td><td>Moon landing</td></tr></tbody>
            </table>
            "#,
        );

        let table = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Table(table) => Some(table),
                _ => None,
            })
            .expect("table node");

        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1].cells[1].text, "Moon landing");
    }

    #[test]
    fn parses_table_rows_inside_wrapper_elements() {
        let nodes = parse_nodes(
            r#"
            <table>
                <tbody>
                    <tr><td>Wrapped row</td></tr>
                </tbody>
            </table>
            "#,
        );

        let table = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Table(table) => Some(table),
                _ => None,
            })
            .expect("table node");

        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells[0].text, "Wrapped row");
    }

    #[test]
    fn keeps_empty_tables_as_table_nodes() {
        let nodes = parse_nodes("<table><tbody><tr><td> </td></tr></tbody></table>");

        assert!(nodes
            .iter()
            .any(|node| matches!(&node.data, Data::Table(_))));
        assert!(!nodes
            .iter()
            .any(|node| matches!(&node.data, Data::Unsupported(UnsupportedElement::Table))));
    }

    #[test]
    fn flattens_nested_inline_content_inside_cells() {
        let nodes = parse_nodes(
            r#"
            <table>
                <tr>
                    <td><a href="/wiki/Earth" rel="mw:WikiLink" title="Earth">Earth</a><br><b>planet</b></td>
                </tr>
            </table>
            "#,
        );

        let table = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Table(table) => Some(table),
                _ => None,
            })
            .expect("table node");

        assert_eq!(table.rows[0].cells[0].text, "Earth planet");
    }

    #[test]
    fn skips_empty_table_rows() {
        let nodes = parse_nodes("<table><tr><td> </td></tr><tr><td>Value</td></tr></table>");

        let table = nodes
            .iter()
            .find_map(|node| match &node.data {
                Data::Table(table) => Some(table),
                _ => None,
            })
            .expect("table node");

        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells[0].text, "Value");
    }
}

impl Parser for WikipediaParser {
    fn parse_document(document: &str, endpoint: Endpoint, language: Language) -> Self {
        let mut parser = WikipediaParser {
            nodes: Vec::new(),
            endpoint,
            language,
        };

        let rc_dom = parse_document(RcDom::default(), Default::default()).one(document);
        parser.parse_node(&rc_dom.document, None, None);

        parser
    }

    fn nodes(self) -> Vec<Raw> {
        self.nodes
    }
}
