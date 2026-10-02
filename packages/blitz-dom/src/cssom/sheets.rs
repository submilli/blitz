//! Constructed sheets and their application to document and shadow scopes.
use super::*;
use style::media_queries::MediaList;
use style::stylesheets::{OriginSet, Stylesheet, StylesheetContents};

impl CssomSheet {
    /// Host-authorized readability captured with this sheet object.
    pub fn origin_clean(&self) -> bool {
        self.origin_clean
    }
    /// Opaque identity valid for the lifetime of the retained sheet.
    pub fn identity(&self) -> usize {
        (&*self.sheet.0 as *const _) as usize
    }
}

impl BaseDocument {
    /// The authoritative parser base captured when this sheet was loaded.
    pub fn retained_stylesheet_url(&self, handle: &CssomSheet) -> Option<String> {
        Some(
            self.retained_sheet(handle)?
                .contents(&self.guard.read())
                .url_data
                .as_str()
                .to_owned(),
        )
    }

    /// Read owner attributes only when applying a new sheet or an attribute edit.
    /// Detached retained objects keep their own media even if the old owner changes.
    pub(crate) fn update_owner_stylesheet_media(&mut self, owner: NodeId) {
        let Some(sheet) = self.retain_stylesheet(owner) else {
            return;
        };
        let media = self.nodes[owner]
            .attr(markup5ever::local_name!("media"))
            .unwrap_or("")
            .to_owned();
        let _ = self.set_stylesheet_media(&sheet, &media);
    }

    /// Construct an initially empty sheet with this document's parser and lock.
    /// Constructed sheets cannot initiate import loads.
    pub fn create_constructed_stylesheet(&self) -> CssomSheet {
        let sheet = Stylesheet::from_str(
            "",
            self.url.url_extra_data(),
            Origin::Author,
            ServoArc::new(self.guard.wrap(MediaList::empty())),
            self.guard.clone(),
            None,
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::No,
        );
        CssomSheet {
            document: self.id(),
            owner: self.root_node().id,
            sheet: DocumentStyleSheet(ServoArc::new(sheet)),
            constructed: true,
            origin_clean: true,
        }
    }

    /// Replace a constructed sheet in place; existing rule handles remain detached.
    pub fn retained_stylesheet_replace(
        &mut self,
        handle: &CssomSheet,
        css: &str,
    ) -> Result<(), CssomError> {
        self.retained_sheet(handle).ok_or(CssomError::NotFound)?;
        if !handle.constructed {
            return Err(CssomError::NotSupported);
        }
        let contents = StylesheetContents::from_str(
            crate::css_limits::bounded(css),
            handle.sheet.contents(&self.guard.read()).url_data.clone(),
            Origin::Author,
            &self.guard,
            None,
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::No,
            None,
        );
        *handle.sheet.0.contents.write_with(&mut self.guard.write()) = contents;
        self.invalidate_cssom_sheet(handle);
        Ok(())
    }

    pub fn stylesheet_disabled(&self, handle: &CssomSheet) -> Option<bool> {
        Some(self.retained_sheet(handle)?.0.disabled())
    }
    pub fn set_stylesheet_disabled(
        &mut self,
        handle: &CssomSheet,
        value: bool,
    ) -> Result<(), CssomError> {
        let sheet = self.retained_sheet(handle).ok_or(CssomError::NotFound)?;
        if sheet.0.set_disabled(value) {
            self.invalidate_cssom_sheet(handle);
        }
        Ok(())
    }
    pub fn stylesheet_media(&self, handle: &CssomSheet) -> Option<String> {
        Some(
            self.retained_sheet(handle)?
                .0
                .media
                .read_with(&self.guard.read())
                .to_css_string(),
        )
    }
    pub fn set_stylesheet_media(
        &mut self,
        handle: &CssomSheet,
        value: &str,
    ) -> Result<(), CssomError> {
        self.retained_sheet(handle).ok_or(CssomError::NotFound)?;
        let url = self.url.url_extra_data();
        let mut context = ParserContext::new(
            Origin::Author,
            &url,
            None,
            ParsingMode::DEFAULT,
            QuirksMode::NoQuirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        let value = if crate::css_limits::allowed(value) {
            value
        } else {
            "not all"
        };
        let mut input = ParserInput::new(value);
        let media = MediaList::parse(&mut context, &mut Parser::new(&mut input));
        *handle.sheet.0.media.write_with(&mut self.guard.write()) = media;
        self.invalidate_cssom_sheet(handle);
        Ok(())
    }

    /// Individual parsed media queries, including commas inside conditions.
    pub fn stylesheet_media_items(&self, handle: &CssomSheet) -> Option<Vec<String>> {
        Some(
            self.retained_sheet(handle)?
                .0
                .media
                .read_with(&self.guard.read())
                .media_queries
                .iter()
                .map(ToCss::to_css_string)
                .collect(),
        )
    }
    pub fn append_stylesheet_medium(
        &mut self,
        handle: &CssomSheet,
        value: &str,
    ) -> Result<(), CssomError> {
        self.change_stylesheet_medium(handle, value, false)
    }
    pub fn delete_stylesheet_medium(
        &mut self,
        handle: &CssomSheet,
        value: &str,
    ) -> Result<(), CssomError> {
        self.change_stylesheet_medium(handle, value, true)
    }
    fn change_stylesheet_medium(
        &mut self,
        handle: &CssomSheet,
        value: &str,
        delete: bool,
    ) -> Result<(), CssomError> {
        self.retained_sheet(handle).ok_or(CssomError::NotFound)?;
        if !crate::css_limits::allowed(value) {
            return Ok(());
        }
        let url = self.url.url_extra_data();
        let context = ParserContext::new(
            Origin::Author,
            &url,
            None,
            ParsingMode::DEFAULT,
            QuirksMode::NoQuirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        let Ok(query) = parser
            .parse_entirely(|parser| style::media_queries::MediaQuery::parse(&context, parser))
        else {
            return Ok(());
        };
        {
            let mut guard = self.guard.write();
            let list = &mut handle.sheet.0.media.write_with(&mut guard).media_queries;
            let position = list.iter().position(|item| item == &query);
            if delete {
                if position.is_none() {
                    return Err(CssomError::NotFound);
                }
                list.retain(|item| item != &query);
            } else {
                if position.is_some() {
                    return Ok(());
                }
                if list.len() >= 4096 {
                    return Err(CssomError::NotSupported);
                }
                list.push(query);
            }
        }
        self.invalidate_cssom_sheet(handle);
        Ok(())
    }

    /// Replace the constructed sheets of the document or a shadow root atomically.
    /// Foreign-document and owner-node sheets are rejected before any mutation.
    pub fn adopt_stylesheets(
        &mut self,
        root: NodeId,
        sheets: &[CssomSheet],
    ) -> Result<(), CssomError> {
        if sheets.len() > 4096 {
            return Err(CssomError::NotSupported);
        }
        if sheets
            .iter()
            .any(|sheet| sheet.document != self.id() || !sheet.constructed)
        {
            return Err(CssomError::NotSupported);
        }
        if root == self.root_node().id {
            let guard = self.guard.read();
            for sheet in self.adopted_stylesheets.drain(..) {
                self.stylist.remove_stylesheet(sheet, &guard);
            }
            for sheet in sheets {
                self.stylist.append_stylesheet(sheet.sheet.clone(), &guard);
                self.adopted_stylesheets.push(sheet.sheet.clone());
            }
            self.stylist
                .force_stylesheet_origins_dirty(OriginSet::all());
        } else {
            let node = self.nodes.get_mut(root).ok_or(CssomError::NotFound)?;
            if node.shadow_root_data.is_none() {
                return Err(CssomError::NotSupported);
            }
            let styles = node.shadow_styles.get_or_insert_with(Default::default);
            styles.adopted = sheets.iter().map(|sheet| sheet.sheet.clone()).collect();
            styles.dirty = true;
        }
        Ok(())
    }

    /// In-place changes must rebuild both ordinary and adopted cascade data.
    /// Detached handles never gain an application scope through mutation.
    pub(super) fn invalidate_cssom_sheet(&mut self, handle: &CssomSheet) {
        self.stylist
            .force_stylesheet_origins_dirty(OriginSet::all());
        for (_, node) in self.nodes.iter_mut() {
            if let Some(styles) = node.shadow_styles.as_mut()
                && styles
                    .sheets
                    .values()
                    .chain(styles.adopted.iter())
                    .any(|sheet| ServoArc::ptr_eq(&sheet.0, &handle.sheet.0))
            {
                styles.dirty = true;
            }
        }
    }
}
