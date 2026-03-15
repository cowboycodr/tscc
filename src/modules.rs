use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::lexer::scanner::Scanner;
use crate::parser::ast::{Program, StmtKind};
use crate::parser::parser::Parser;

#[derive(Debug)]
pub struct ModuleInfo {
    pub path: PathBuf,
    pub source: String,
    pub program: Program,
}

#[derive(Debug)]
pub struct ModuleGraph {
    /// All modules in dependency order (dependencies first)
    pub modules: Vec<ModuleInfo>,
    /// Map from canonical path to index in `modules`
    path_to_idx: HashMap<PathBuf, usize>,
}

impl ModuleGraph {
    /// Build a module graph starting from the entry file.
    /// Returns modules in topological order (dependencies before dependents).
    pub fn build(entry: &Path) -> Result<Self, String> {
        let mut graph = ModuleGraph {
            modules: Vec::new(),
            path_to_idx: HashMap::new(),
        };

        let entry_canonical = std::fs::canonicalize(entry)
            .map_err(|e| format!("Cannot resolve '{}': {}", entry.display(), e))?;

        let mut visited = HashSet::new();
        let mut order = Vec::new();

        graph.visit(&entry_canonical, &mut visited, &mut order)?;

        // Reorder modules: dependencies first, entry last
        let mut ordered_modules = Vec::new();
        let mut new_indices = HashMap::new();
        for path in &order {
            let idx = graph.path_to_idx[path];
            new_indices.insert(path.clone(), ordered_modules.len());
            // Take the module out via swap
            ordered_modules.push(std::mem::replace(
                &mut graph.modules[idx],
                ModuleInfo {
                    path: PathBuf::new(),
                    source: String::new(),
                    program: Program {
                        statements: Vec::new(),
                    },
                },
            ));
        }

        graph.modules = ordered_modules;
        graph.path_to_idx = new_indices;

        Ok(graph)
    }

    fn visit(
        &mut self,
        path: &Path,
        visited: &mut HashSet<PathBuf>,
        order: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        if visited.contains(path) {
            return Ok(());
        }
        visited.insert(path.to_path_buf());

        // Parse this module
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("Error reading '{}': {}", path.display(), e))?;

        let tokens = Scanner::new(&source)
            .scan_tokens()
            .map_err(|e| format!("Lexer error in '{}': {}", path.display(), e.message))?;

        let mut parser = Parser::new(tokens);
        let program = parser
            .parse()
            .map_err(|e| format!("Parse error in '{}': {}", path.display(), e.message))?;

        let idx = self.modules.len();
        self.path_to_idx.insert(path.to_path_buf(), idx);

        // Discover imports
        let parent_dir = path.parent().unwrap_or(Path::new("."));
        let mut import_paths = Vec::new();

        for stmt in &program.statements {
            if let StmtKind::Import { source, .. } = &stmt.kind {
                let dep_path = resolve_import(parent_dir, source)?;
                import_paths.push(dep_path);
            }
        }

        self.modules.push(ModuleInfo {
            path: path.to_path_buf(),
            source,
            program,
        });

        // Visit dependencies first (DFS)
        for dep_path in import_paths {
            self.visit(&dep_path, visited, order)?;
        }

        // Add this module after its dependencies
        order.push(path.to_path_buf());

        Ok(())
    }

    pub fn entry_index(&self) -> usize {
        self.modules.len() - 1
    }
}

/// Resolve a relative import path to an absolute filesystem path.
/// Appends .ts if the path doesn't already have an extension.
fn resolve_import(parent_dir: &Path, source: &str) -> Result<PathBuf, String> {
    let mut target = parent_dir.join(source);

    // If no extension, try .ts
    if target.extension().is_none() {
        target.set_extension("ts");
    }

    if !target.exists() {
        // Also try without extension change
        let alt = parent_dir.join(source);
        if alt.exists() {
            return std::fs::canonicalize(&alt)
                .map_err(|e| format!("Cannot resolve '{}': {}", source, e));
        }
        return Err(format!(
            "Cannot find module '{}' (resolved to '{}')",
            source,
            target.display()
        ));
    }

    std::fs::canonicalize(&target).map_err(|e| format!("Cannot resolve '{}': {}", source, e))
}
