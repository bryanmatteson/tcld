use super::pass_trait::DecompilerPass;
use std::sync::{Arc, Mutex, OnceLock};

/// Global registry for custom decompilation passes.
///
/// Passes registered here are appended to the built-in pipeline. They run
/// after all built-in passes, ordered by priority.
#[derive(Clone)]
pub struct PassRegistry {
    passes: Vec<Arc<dyn DecompilerPass>>,
}

impl PassRegistry {
    pub fn new() -> Self {
        PassRegistry { passes: Vec::new() }
    }

    /// Register a custom pass.
    pub fn register(&mut self, pass: Arc<dyn DecompilerPass>) {
        self.passes.push(pass);
        self.passes.sort_by_key(|p| p.priority());
    }

    /// Get all registered custom passes, in priority order.
    pub fn passes(&self) -> &[Arc<dyn DecompilerPass>] {
        &self.passes
    }

    /// Remove all registered custom passes.
    pub fn clear(&mut self) {
        self.passes.clear();
    }
}

impl Default for PassRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_REGISTRY: OnceLock<Mutex<PassRegistry>> = OnceLock::new();

/// Access the global pass registry.
pub fn global_registry() -> &'static Mutex<PassRegistry> {
    GLOBAL_REGISTRY.get_or_init(|| Mutex::new(PassRegistry::new()))
}

/// Convenience: register a pass in the global registry.
///
/// Global passes run only for pipelines configured with
/// `DecompilePipelineConfig::with_global_registry`.
pub fn register_pass(pass: Arc<dyn DecompilerPass>) {
    global_registry().lock().unwrap().register(pass);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::ByteCode;
    use crate::error::Result;
    use std::ops::Range;

    struct TestPass {
        name: String,
        priority: u32,
    }

    impl DecompilerPass for TestPass {
        fn name(&self) -> &str {
            &self.name
        }

        fn run(
            &self,
            _graph: &mut crate::code_graph::CodeGraph,
            _bytecode: &ByteCode,
            _range: Range<i32>,
        ) -> Result<bool> {
            Ok(false)
        }

        fn priority(&self) -> u32 {
            self.priority
        }
    }

    #[test]
    fn registry_sorts_passes_by_priority() {
        let mut registry = PassRegistry::new();
        registry.register(Arc::new(TestPass {
            name: "pass_a".to_string(),
            priority: 100,
        }));
        registry.register(Arc::new(TestPass {
            name: "pass_c".to_string(),
            priority: 300,
        }));
        registry.register(Arc::new(TestPass {
            name: "pass_b".to_string(),
            priority: 200,
        }));

        let passes = registry.passes();
        assert_eq!(passes.len(), 3);
        assert_eq!(passes[0].name(), "pass_a");
        assert_eq!(passes[1].name(), "pass_b");
        assert_eq!(passes[2].name(), "pass_c");
    }

    #[test]
    fn registry_can_be_cleared() {
        let mut registry = PassRegistry::new();
        registry.register(Arc::new(TestPass {
            name: "test".to_string(),
            priority: 100,
        }));
        assert_eq!(registry.passes().len(), 1);

        registry.clear();
        assert_eq!(registry.passes().len(), 0);
    }
}
