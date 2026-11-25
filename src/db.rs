use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use tempfile::NamedTempFile;

use crate::document::{page_id, Document, RemoteDocument};
use crate::notion::PageInfo;
use crate::review::Snapshot;

/// Sled's final background file handle can briefly outlive the last store.
/// Preserve exclusive locking; wait only for WouldBlock, never other errors.
fn open_with_retry<T>(mut open: impl FnMut() -> sled::Result<T>) -> sled::Result<T> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        match open() {
            Err(error) if lock_contention(&error) => {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return Err(error);
                }
                std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
            }
            result => return result,
        }
    }
}

fn lock_contention(error: &sled::Error) -> bool {
    let sled::Error::Io(io) = error else {
        return false;
    };
    if io.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    // Sled 0.34.7 config::open_file erases the OS kind into this specific
    // Other-error wrapper. Do not retry arbitrary Other or permission errors.
    let message = io.to_string();
    io.kind() == std::io::ErrorKind::Other
        && message.starts_with("could not acquire lock on ")
        && message
            .rsplit_once(": Os { ")
            .is_some_and(|(_, os)| os.contains(", kind: WouldBlock, ") && os.ends_with(" }"))
}

pub struct Store {
    root: PathBuf,
    db: sled::Db,
    documents: sled::Tree,
    #[cfg(test)]
    fail_journal_state: Option<crate::journal::State>,
}

impl Store {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn cache_save<T: serde::Serialize>(
        &self,
        tree: &str,
        key: &str,
        value: &T,
    ) -> Result<()> {
        self.db
            .open_tree(tree)?
            .insert(key.as_bytes(), serde_json::to_vec(value)?)?;
        self.db.flush()?;
        Ok(())
    }

    pub(crate) fn cache_get<T: serde::de::DeserializeOwned>(
        &self,
        tree: &str,
        key: &str,
    ) -> Result<T> {
        let bytes = self
            .db
            .open_tree(tree)?
            .get(key.as_bytes())?
            .context("no cached snapshot; pull it first")?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub(crate) fn journal_id(&self) -> Result<u64> {
        Ok(self.db.generate_id()?)
    }

    pub(crate) fn write_operation(&self, operation: &crate::journal::Operation) -> Result<()> {
        #[cfg(test)]
        ensure!(
            self.fail_journal_state != Some(operation.state),
            "injected journal storage failure"
        );
        self.db
            .open_tree("operations-v1")?
            .insert(operation.id.to_be_bytes(), serde_json::to_vec(operation)?)?;
        self.db
            .flush()
            .context("flush operation journal before proceeding")?;
        Ok(())
    }

    pub fn operation(&self, id: u64) -> Result<crate::journal::Operation> {
        let data = self
            .db
            .open_tree("operations-v1")?
            .get(id.to_be_bytes())?
            .context("operation not found")?;
        let op: crate::journal::Operation = serde_json::from_slice(&data)?;
        ensure!(op.id == id, "operation identity mismatch");
        Ok(op)
    }

    pub fn operations(&self) -> Result<Vec<crate::journal::Operation>> {
        self.db
            .open_tree("operations-v1")?
            .iter()
            .map(|entry| {
                let (key, value) = entry?;
                let op: crate::journal::Operation = serde_json::from_slice(&value)?;
                ensure!(
                    key.as_ref() == op.id.to_be_bytes(),
                    "operation identity mismatch"
                );
                Ok(op)
            })
            .collect()
    }

    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join("docs")).context("create document directory")?;
        let root = fs::canonicalize(root).context("resolve workspace")?;
        let db = open_with_retry(|| sled::open(root.join("store")))
            .context("open sled workspace (one CLI at a time)")?;
        let documents = db.open_tree("documents-v1")?;
        Ok(Self {
            root,
            db,
            documents,
            #[cfg(test)]
            fail_journal_state: None,
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

    pub fn metadata(&self, id: &str) -> Result<PageInfo> {
        let id = page_id(id)?;
        let data = self
            .db
            .open_tree("page-metadata-v1")?
            .get(id.as_bytes())?
            .context("no cached page metadata; run inspect online first")?;
        let info: PageInfo = serde_json::from_slice(&data)?;
        ensure!(info.id == id, "metadata identity mismatch");
        Ok(info)
    }

    /// Refresh metadata without advancing a document's body baseline.
    pub fn save_metadata(&self, info: &PageInfo) -> Result<()> {
        ensure!(
            page_id(&info.id)? == info.id,
            "metadata UUID must be canonical"
        );
        self.db
            .open_tree("page-metadata-v1")?
            .insert(info.id.as_bytes(), serde_json::to_vec(info)?)?;
        self.db.flush()?;
        if self.contains(&info.id)? {
            let previous = self.get(&info.id)?;
            let mut next = previous.clone();
            next.title = info.title.clone();
            next.url = info.url.clone();
            next.last_edited_time = info.last_edited_time.clone();
            self.save(&next, Some(&previous))?;
        }
        Ok(())
    }

    /// Flush a complete recovery record before changing any local document.
    pub fn archive(
        &self,
        doc: &Document,
        remote: Option<RemoteDocument>,
        action: &str,
        selected_markdown: &str,
    ) -> Result<Snapshot> {
        let snapshot = Snapshot {
            id: self.db.generate_id()?,
            action: action.to_owned(),
            local: doc.clone(),
            remote,
            selected_markdown: selected_markdown.to_owned(),
        };
        let snapshots = self.db.open_tree("reconciliation-v1")?;
        snapshots.insert(snapshot.id.to_be_bytes(), serde_json::to_vec(&snapshot)?)?;
        self.db
            .flush()
            .context("flush recovery snapshot before reconciliation")?;
        Ok(snapshot)
    }

    pub fn snapshot(&self, id: u64) -> Result<Snapshot> {
        let tree = self.db.open_tree("reconciliation-v1")?;
        let data = tree
            .get(id.to_be_bytes())?
            .context("recovery snapshot not found")?;
        let snapshot: Snapshot = serde_json::from_slice(&data)?;
        ensure!(snapshot.id == id, "snapshot key/identity mismatch");
        Ok(snapshot)
    }

    pub fn snapshots(&self, page: &str) -> Result<Vec<Snapshot>> {
        let page = page_id(page)?;
        self.db
            .open_tree("reconciliation-v1")?
            .iter()
            .map(|entry| {
                let (_, data) = entry?;
                let snapshot: Snapshot = serde_json::from_slice(&data)?;
                Ok(snapshot)
            })
            .filter_map(|entry: Result<Snapshot>| match entry {
                Ok(snapshot) if snapshot.local.id != page => None,
                other => Some(other),
            })
            .collect()
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

#[cfg(test)]
mod journal_failures {
    use super::*;
    use crate::{
        commands,
        journal::State,
        notion::{CreateParent, Notion},
    };

    #[test]
    fn open_retries_only_lock_contention() {
        let mut attempts = 0;
        open_with_retry(|| {
            attempts += 1;
            if attempts < 3 {
                Err(sled::Error::Io(std::io::Error::from(
                    std::io::ErrorKind::WouldBlock,
                )))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(attempts, 3);
        let mut attempts = 0;
        assert!(open_with_retry::<()>(|| {
            attempts += 1;
            Err(sled::Error::Io(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied,
            )))
        })
        .is_err());
        assert_eq!(attempts, 1);
        assert!(lock_contention(&sled::Error::Io(std::io::Error::other("could not acquire lock on \"/tmp/store/db\": Os { code: 11, kind: WouldBlock, message: \"Resource temporarily unavailable\" }"))));
        assert!(!lock_contention(&sled::Error::Io(std::io::Error::other(
            "some unrelated WouldBlock message"
        ))));
    }

    #[test]
    fn real_store_reopen_is_reliable_and_second_holder_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        for _ in 0..30 {
            let store = Store::open(dir.path()).unwrap();
            let op = store
                .prepare(
                    crate::journal::Intent::Create {
                        parent: None,
                        title: "Notes".into(),
                        markdown: "body".into(),
                    },
                    None,
                )
                .unwrap();
            assert_eq!(
                store.operation(op.id).unwrap().state,
                crate::journal::State::Prepared
            );
        }
        let holder = Store::open(dir.path()).unwrap();
        let start = std::time::Instant::now();
        assert!(Store::open(dir.path()).is_err());
        assert!(start.elapsed() >= std::time::Duration::from_millis(250));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        drop(holder);
        assert!(Store::open(dir.path()).is_ok());
    }

    #[tokio::test]
    async fn failed_preparation_or_sent_flush_prevents_mutation() {
        for failure in [State::Prepared, State::Sent] {
            let dir = tempfile::TempDir::new().unwrap();
            let mut store = Store::open(dir.path()).unwrap();
            store.fail_journal_state = Some(failure);
            let server = wiremock::MockServer::start().await;
            let notion = Notion::new("test", &format!("{}/v1", server.uri())).unwrap();
            let error = commands::create_document(
                &store,
                &notion,
                &CreateParent::Workspace,
                "Notes",
                "authored",
            )
            .await
            .unwrap_err();
            assert!(format!("{error:#}").contains("injected journal storage failure"));
            assert!(server.received_requests().await.unwrap().is_empty());
        }
    }
}
