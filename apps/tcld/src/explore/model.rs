use crate::cli::ExploreArgs;
use crate::error::Result;
use std::ops::{Deref, DerefMut};
use std::path::Path;

pub use tcld_explore::ExplorerView;
use tcld_explore::{Diagnostic, ExploreOptions};

pub struct ExplorerCore {
    inner: tcld_explore::ExplorerCore,
    no_colors: bool,
    theme: Option<String>,
    highlight: crate::highlight::HighlightCache,
}

impl ExplorerCore {
    pub fn new(args: ExploreArgs) -> Result<Self> {
        let inner = tcld_explore::ExplorerCore::new(ExploreOptions {
            input_path: args.input_path,
            text_truncate: None,
            page_size: args.page_size,
        })?;
        Ok(Self {
            inner,
            no_colors: args.no_colors,
            theme: args.theme,
            highlight: Default::default(),
        })
    }

    pub fn from_snapshot(args: ExploreArgs, snapshot: tcld_decompile::AnalysisSnapshot) -> Self {
        let inner = tcld_explore::ExplorerCore::from_snapshot(
            ExploreOptions {
                input_path: args.input_path,
                text_truncate: None,
                page_size: args.page_size,
            },
            snapshot,
        );
        Self {
            inner,
            no_colors: args.no_colors,
            theme: args.theme,
            highlight: Default::default(),
        }
    }

    /// The Tree-sitter parse cache backing document highlighting.
    pub fn highlight(&self) -> &crate::highlight::HighlightCache {
        &self.highlight
    }

    pub fn no_colors(&self) -> bool {
        self.no_colors
    }

    pub fn theme(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    pub(crate) fn core(&self) -> &tcld_explore::ExplorerCore {
        &self.inner
    }

    pub(crate) fn core_mut(&mut self) -> &mut tcld_explore::ExplorerCore {
        &mut self.inner
    }

    pub fn export_generated_source(
        &self,
        path: impl AsRef<Path>,
    ) -> std::result::Result<tcld_explore::ExportResult, Diagnostic> {
        self.inner.export_generated_source(path)
    }
}

impl Deref for ExplorerCore {
    type Target = tcld_explore::ExplorerCore;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for ExplorerCore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
