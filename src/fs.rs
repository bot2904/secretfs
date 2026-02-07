use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyWrite, Request, TimeOrNow,
};
use log::{debug, error, warn};

use crate::handle::HandleTable;
use crate::transformer::Transformer;

const TTL: Duration = Duration::from_secs(1);

/// The SecretFS FUSE filesystem.
pub struct SecretFs {
    /// The source directory being mirrored.
    source: PathBuf,
    /// Handle table for open files.
    handles: HandleTable,
    /// Transformer for computing sizes in getattr.
    transformer: Transformer,
    /// Inode → real path cache. Populated on lookup/readdir.
    inode_cache: Mutex<HashMap<u64, PathBuf>>,
}

impl SecretFs {
    pub fn new(source: PathBuf, transformer: Transformer) -> Self {
        let handles = HandleTable::new(transformer.clone());

        // Pre-populate the root inode
        let mut cache = HashMap::new();
        if let Ok(meta) = std::fs::metadata(&source) {
            cache.insert(meta.ino(), source.clone());
        }
        // Also map FUSE root inode 1 → source
        cache.insert(1, source.clone());

        Self {
            source,
            handles,
            transformer,
            inode_cache: Mutex::new(cache),
        }
    }

    /// Look up the real path for an inode from our cache.
    fn real_path(&self, ino: u64) -> Option<PathBuf> {
        self.inode_cache.lock().unwrap().get(&ino).cloned()
    }

    /// Register an inode → path mapping in the cache.
    fn cache_inode(&self, ino: u64, path: PathBuf) {
        self.inode_cache.lock().unwrap().insert(ino, path);
    }

    /// Remove an inode from the cache.
    fn uncache_inode(&self, ino: u64) {
        self.inode_cache.lock().unwrap().remove(&ino);
    }

    /// Convert real filesystem metadata to FUSE FileAttr.
    /// If `transformed_size` is provided, use it; otherwise compute it for regular files.
    fn meta_to_attr(&self, meta: &std::fs::Metadata, ino: u64, path: Option<&PathBuf>) -> FileAttr {
        let kind = if meta.is_dir() {
            FileType::Directory
        } else if meta.is_symlink() {
            FileType::Symlink
        } else {
            FileType::RegularFile
        };

        // For regular files, compute the transformed size
        let size = if meta.is_file() {
            if let Some(p) = path {
                if let Ok(content) = std::fs::read(p) {
                    let transformed = self.transformer.filter_read(&content);
                    transformed.len() as u64
                } else {
                    meta.len()
                }
            } else {
                meta.len()
            }
        } else {
            meta.len()
        };

        FileAttr {
            ino,
            size,
            blocks: meta.blocks(),
            atime: system_time_from_epoch(meta.atime(), meta.atime_nsec()),
            mtime: system_time_from_epoch(meta.mtime(), meta.mtime_nsec()),
            ctime: system_time_from_epoch(meta.ctime(), meta.ctime_nsec()),
            crtime: SystemTime::UNIX_EPOCH,
            kind,
            perm: meta.mode() as u16,
            nlink: meta.nlink() as u32,
            uid: meta.uid(),
            gid: meta.gid(),
            rdev: meta.rdev() as u32,
            blksize: meta.blksize() as u32,
            flags: 0,
        }
    }
}

fn system_time_from_epoch(secs: i64, nsecs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::new(secs as u64, nsecs as u32)
    } else {
        UNIX_EPOCH
    }
}

impl Filesystem for SecretFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        debug!("lookup: parent={}, name={:?}", parent, name);

        let parent_path = match self.real_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let child_path = parent_path.join(name);
        match std::fs::symlink_metadata(&child_path) {
            Ok(meta) => {
                let ino = meta.ino();
                self.cache_inode(ino, child_path.clone());
                let attr = self.meta_to_attr(&meta, ino, Some(&child_path));
                reply.entry(&TTL, &attr, 0);
            }
            Err(e) => {
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        debug!("getattr: ino={}", ino);

        let path = match self.real_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                let attr = self.meta_to_attr(&meta, ino, Some(&path));
                reply.attr(&TTL, &attr);
            }
            Err(e) => {
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
            }
        }
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        debug!("setattr: ino={}", ino);

        let path = match self.real_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Handle chmod
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode);
            if let Err(e) = std::fs::set_permissions(&path, perms) {
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
                return;
            }
        }

        // Handle truncate
        if let Some(size) = size {
            if size == 0 {
                if let Err(e) = std::fs::write(&path, b"") {
                    reply.error(e.raw_os_error().unwrap_or(libc::EIO));
                    return;
                }
            }
        }

        // Handle chown
        if uid.is_some() || gid.is_some() {
            use std::ffi::CString;
            let c_path = match CString::new(path.as_os_str().as_bytes()) {
                Ok(c) => c,
                Err(_) => {
                    reply.error(libc::EINVAL);
                    return;
                }
            };
            let uid_val = uid.map(|u| u as libc::uid_t).unwrap_or(u32::MAX);
            let gid_val = gid.map(|g| g as libc::gid_t).unwrap_or(u32::MAX);
            unsafe {
                if libc::chown(c_path.as_ptr(), uid_val, gid_val) != 0 {
                    reply.error(*libc::__errno_location());
                    return;
                }
            }
        }

        // Re-read attributes after modifications
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                let attr = self.meta_to_attr(&meta, ino, Some(&path));
                reply.attr(&TTL, &attr);
            }
            Err(e) => {
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir: ino={}, offset={}", ino, offset);

        let path = match self.real_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let mut entries = vec![];

        // Add . and ..
        if let Ok(meta) = std::fs::metadata(&path) {
            entries.push((meta.ino(), FileType::Directory, ".".to_string()));
        }
        // For .., use parent's inode or self if root
        let parent_ino = if path == self.source {
            1 // FUSE root
        } else if let Some(parent) = path.parent() {
            if let Ok(meta) = std::fs::metadata(parent) {
                meta.ino()
            } else {
                1
            }
        } else {
            1
        };
        entries.push((parent_ino, FileType::Directory, "..".to_string()));

        // Add directory entries
        match std::fs::read_dir(&path) {
            Ok(dir) => {
                for entry in dir.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        let kind = if meta.is_dir() {
                            FileType::Directory
                        } else if meta.is_symlink() {
                            FileType::Symlink
                        } else {
                            FileType::RegularFile
                        };
                        let name = entry.file_name().to_string_lossy().to_string();
                        let entry_ino = meta.ino();
                        // Cache the inode mapping
                        self.cache_inode(entry_ino, entry.path());
                        entries.push((entry_ino, kind, name));
                    }
                }
            }
            Err(e) => {
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
                return;
            }
        }

        for (i, (entry_ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
            if reply.add(*entry_ino, (i + 1) as i64, *kind, name) {
                break;
            }
        }

        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("open: ino={}, flags={}", ino, flags);

        let path = match self.real_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        if path.is_dir() {
            reply.error(libc::EISDIR);
            return;
        }

        match std::fs::read(&path) {
            Ok(content) => {
                let fh = self.handles.open(content, flags);
                // FOPEN_DIRECT_IO to prevent kernel caching (we transform content)
                reply.opened(fh, fuser::consts::FOPEN_DIRECT_IO);
            }
            Err(e) => {
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
            }
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read: fh={}, offset={}, size={}", fh, offset, size);

        let handle = match self.handles.get(fh) {
            Some(h) => h,
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        let handle = handle.lock().unwrap();
        let content = &handle.read_content;

        let offset = offset as usize;
        if offset >= content.len() {
            reply.data(&[]);
            return;
        }

        let end = std::cmp::min(offset + size as usize, content.len());
        reply.data(&content[offset..end]);
    }

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        debug!("write: fh={}, offset={}, size={}", fh, offset, data.len());

        let handle = match self.handles.get(fh) {
            Some(h) => h,
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        let mut handle = handle.lock().unwrap();
        let offset = offset as usize;

        // Extend buffer if needed
        if offset + data.len() > handle.write_buf.len() {
            handle.write_buf.resize(offset + data.len(), 0);
        }

        handle.write_buf[offset..offset + data.len()].copy_from_slice(data);
        handle.dirty = true;

        reply.written(data.len() as u32);
    }

    fn flush(&mut self, _req: &Request, ino: u64, fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        debug!("flush: ino={}, fh={}", ino, fh);

        if let Some(write_back) = self.handles.flush(fh) {
            let path = match self.real_path(ino) {
                Some(p) => p,
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            };

            if let Err(e) = std::fs::write(&path, &write_back) {
                error!("Failed to write back to {:?}: {}", path, e);
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
                return;
            }
        }

        reply.ok();
    }

    fn release(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        debug!("release: ino={}, fh={}", ino, fh);

        if let Some(write_back) = self.handles.release(fh) {
            let path = match self.real_path(ino) {
                Some(p) => p,
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            };

            if let Err(e) = std::fs::write(&path, &write_back) {
                error!("Failed to write back to {:?}: {}", path, e);
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
                return;
            }
        }

        reply.ok();
    }

    fn opendir(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("opendir: ino={}", ino);

        let path = match self.real_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        if !path.is_dir() {
            reply.error(libc::ENOTDIR);
            return;
        }

        reply.opened(0, 0);
    }

    fn releasedir(&mut self, _req: &Request, _ino: u64, _fh: u64, _flags: i32, reply: ReplyEmpty) {
        reply.ok();
    }

    fn create(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        debug!("create: parent={}, name={:?}", parent, name);

        let parent_path = match self.real_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let child_path = parent_path.join(name);

        // Create the file
        if let Err(e) = std::fs::write(&child_path, b"") {
            reply.error(e.raw_os_error().unwrap_or(libc::EIO));
            return;
        }

        // Set permissions
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode);
        if let Err(e) = std::fs::set_permissions(&child_path, perms) {
            warn!("Failed to set permissions on {:?}: {}", child_path, e);
        }

        match std::fs::symlink_metadata(&child_path) {
            Ok(meta) => {
                let ino = meta.ino();
                self.cache_inode(ino, child_path.clone());
                let attr = self.meta_to_attr(&meta, ino, Some(&child_path));
                let fh = self.handles.open(Vec::new(), flags);
                reply.created(&TTL, &attr, 0, fh, fuser::consts::FOPEN_DIRECT_IO);
            }
            Err(e) => {
                reply.error(e.raw_os_error().unwrap_or(libc::EIO));
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!("unlink: parent={}, name={:?}", parent, name);

        let parent_path = match self.real_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let child_path = parent_path.join(name);

        // Remove from inode cache before deleting
        if let Ok(meta) = std::fs::symlink_metadata(&child_path) {
            self.uncache_inode(meta.ino());
        }

        match std::fs::remove_file(&child_path) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        debug!("mkdir: parent={}, name={:?}", parent, name);

        let parent_path = match self.real_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let child_path = parent_path.join(name);
        if let Err(e) = std::fs::create_dir(&child_path) {
            reply.error(e.raw_os_error().unwrap_or(libc::EIO));
            return;
        }

        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode);
        let _ = std::fs::set_permissions(&child_path, perms);

        match std::fs::symlink_metadata(&child_path) {
            Ok(meta) => {
                let ino = meta.ino();
                self.cache_inode(ino, child_path.clone());
                let attr = self.meta_to_attr(&meta, ino, Some(&child_path));
                reply.entry(&TTL, &attr, 0);
            }
            Err(e) => reply.error(e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!("rmdir: parent={}, name={:?}", parent, name);

        let parent_path = match self.real_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let child_path = parent_path.join(name);

        // Remove from inode cache
        if let Ok(meta) = std::fs::symlink_metadata(&child_path) {
            self.uncache_inode(meta.ino());
        }

        match std::fs::remove_dir(&child_path) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        debug!(
            "rename: parent={}, name={:?} -> newparent={}, newname={:?}",
            parent, name, newparent, newname
        );

        let parent_path = match self.real_path(parent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        let new_parent_path = match self.real_path(newparent) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let old_path = parent_path.join(name);
        let new_path = new_parent_path.join(newname);

        // Update inode cache after rename
        let ino = std::fs::symlink_metadata(&old_path)
            .ok()
            .map(|m| m.ino());

        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                if let Some(ino) = ino {
                    self.cache_inode(ino, new_path);
                }
                reply.ok();
            }
            Err(e) => reply.error(e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        debug!("readlink: ino={}", ino);

        let path = match self.real_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        match std::fs::read_link(&path) {
            Ok(target) => reply.data(target.as_os_str().as_bytes()),
            Err(e) => reply.error(e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }

    fn access(&mut self, _req: &Request, ino: u64, mask: i32, reply: ReplyEmpty) {
        debug!("access: ino={}, mask={}", ino, mask);

        let path = match self.real_path(ino) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        if path.exists() {
            reply.ok();
        } else {
            reply.error(libc::ENOENT);
        }
    }
}
