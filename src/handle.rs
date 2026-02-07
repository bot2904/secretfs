use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::transformer::Transformer;

/// State for an open file handle.
#[derive(Debug)]
pub struct FileHandle {
    /// The transformed (placeholder-replaced) content served to readers.
    pub read_content: Vec<u8>,
    /// The write buffer, accumulating writes from the consumer.
    /// Starts as a clone of read_content, modified by writes.
    pub write_buf: Vec<u8>,
    /// Whether the file has been written to (dirty).
    pub dirty: bool,
    /// Flags the file was opened with.
    #[allow(dead_code)]
    pub flags: i32,
}

/// Manages all open file handles, keyed by a u64 file handle ID.
pub struct HandleTable {
    handles: Mutex<HashMap<u64, Arc<Mutex<FileHandle>>>>,
    next_fh: Mutex<u64>,
    transformer: Transformer,
}

impl HandleTable {
    pub fn new(transformer: Transformer) -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
            next_fh: Mutex::new(1),
            transformer,
        }
    }

    /// Open a file: read source content, transform it, store in handle table.
    /// Returns the file handle ID.
    pub fn open(&self, source_content: Vec<u8>, flags: i32) -> u64 {
        let transformed = self.transformer.filter_read(&source_content);

        let handle = FileHandle {
            write_buf: transformed.clone(),
            read_content: transformed,
            dirty: false,
            flags,
        };

        let mut next_fh = self.next_fh.lock().unwrap();
        let fh = *next_fh;
        *next_fh += 1;

        self.handles
            .lock()
            .unwrap()
            .insert(fh, Arc::new(Mutex::new(handle)));

        fh
    }

    /// Get a reference to a file handle.
    pub fn get(&self, fh: u64) -> Option<Arc<Mutex<FileHandle>>> {
        self.handles.lock().unwrap().get(&fh).cloned()
    }

    /// Release a file handle. If dirty, returns the reverse-transformed content
    /// that should be written back to the source file.
    pub fn release(&self, fh: u64) -> Option<Vec<u8>> {
        let handle = self.handles.lock().unwrap().remove(&fh)?;
        let handle = handle.lock().unwrap();

        if handle.dirty {
            let restored = self.transformer.filter_write(&handle.write_buf);
            Some(restored)
        } else {
            None
        }
    }

    /// Flush a dirty file handle: returns content to write back if dirty.
    /// Does NOT remove the handle.
    pub fn flush(&self, fh: u64) -> Option<Vec<u8>> {
        let handle_arc = self.handles.lock().unwrap().get(&fh)?.clone();
        let mut handle = handle_arc.lock().unwrap();

        if handle.dirty {
            let restored = self.transformer.filter_write(&handle.write_buf);
            handle.dirty = false;
            Some(restored)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SecretDef;
    use crate::mapper::SecretMapper;

    #[test]
    fn test_open_read_write_release() {
        let defs = vec![SecretDef::Literal {
            literal: "my-secret".to_string(),
        }];
        let mapper = SecretMapper::new();
        let transformer = Transformer::new(&defs, mapper);
        let table = HandleTable::new(transformer);

        let content = b"password is my-secret here".to_vec();
        let fh = table.open(content, 0);

        // Read should have placeholder
        {
            let handle = table.get(fh).unwrap();
            let handle = handle.lock().unwrap();
            let text = std::str::from_utf8(&handle.read_content).unwrap();
            assert!(!text.contains("my-secret"));
            assert!(text.contains("<|SECRET:0001|>"));
        }

        // Simulate a write that keeps the placeholder
        {
            let handle = table.get(fh).unwrap();
            let mut handle = handle.lock().unwrap();
            handle.write_buf = b"modified: <|SECRET:0001|> end".to_vec();
            handle.dirty = true;
        }

        // Release should reverse-transform
        let restored = table.release(fh).unwrap();
        let text = std::str::from_utf8(&restored).unwrap();
        assert_eq!(text, "modified: my-secret end");
    }
}
