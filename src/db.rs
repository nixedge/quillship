use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use tempfile::NamedTempFile;

use crate::document::{page_id, Document};

pub struct Store {
    root: PathBuf,
    db: sled::Db,
    documents: sled::Tree,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join("docs")).context("create document directory")?;
        let root = fs::canonicalize(root).context("resolve workspace")?;
        let db =
            sled::open(root.join("store")).context("open sled workspace (one CLI at a time)")?;
        let documents = db.open_tree("documents-v1")?;
        Ok(Self {
            root,
            db,
            documents,
        })
    }

    pub fn path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.root.join("docs").join(format!("{}.md", page_id(id)?)))
    }

    fn record(&self, doc: &Document) -> Result<()> {
        self.documents
            .insert(doc.id.as_bytes(), serde_json::to_vec(doc)?)?;
        self.db.flush().context("flush local documents")?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Document> {
        let id = page_id(id)?;
        let data = self
            .documents
            .get(id.as_bytes())?
            .with_context(|| format!("page {id} is not local; pull it first"))?;
        let mut doc: Document = serde_json::from_slice(&data).context("corrupt document record")?;
        ensure!(doc.id == id, "document key/identity mismatch");
        // Never treat a missing or unreadable file as an empty document.
        let markdown = fs::read_to_string(self.path(&id)?)
            .with_context(|| format!("read working file for {id}; stored body remains in sled"))?;
        if markdown != doc.markdown {
            doc.markdown = markdown;
            self.record(&doc)?;
        }
        Ok(doc)
    }

    pub fn contains(&self, id: &str) -> Result<bool> {
        Ok(self.documents.contains_key(page_id(id)?.as_bytes())?)
    }

    pub fn all(&self) -> Result<Vec<Document>> {
        // Collect keys before ingesting working copies; do not mutate an iterator.
        let ids = self
            .documents
            .iter()
            .keys()
            .map(|key| String::from_utf8(key?.to_vec()).context("invalid document key"))
            .collect::<Result<Vec<_>>>()?;
        let mut docs = ids
            .iter()
            .map(|id| self.get(id))
            .collect::<Result<Vec<_>>>()?;
        docs.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        Ok(docs)
    }

    /// Compare the working file to the snapshot before overwriting it.
    /// This detects external edits made while a network operation was running.
    pub fn save(&self, doc: &Document, expected: Option<&Document>) -> Result<()> {
        let id = page_id(&doc.id)?;
        ensure!(id == doc.id, "stored page UUID must be canonical");
        let path = self.path(&id)?;
        if let Some(previous) = expected {
            ensure!(previous.id == id, "document identity changed");
            ensure!(
                fs::read_to_string(&path)? == previous.markdown,
                "working file changed during operation for {id}; file preserved; run command again"
            );
        } else {
            ensure!(!self.contains(&id)?, "document already exists: {id}");
            ensure!(
                !path.exists(),
                "untracked working file exists at {}; file preserved",
                path.display()
            );
        }
        if expected.is_none_or(|previous| previous.markdown != doc.markdown) {
            let mut tmp = NamedTempFile::new_in(self.root.join("docs"))?;
            tmp.write_all(doc.markdown.as_bytes())?;
            tmp.as_file().sync_all()?;
            tmp.persist(&path)
                .context("replace working file atomically")?;
            fs::File::open(self.root.join("docs"))?.sync_all()?;
        }
        self.record(doc)
    }
}
