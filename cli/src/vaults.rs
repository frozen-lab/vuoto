use crate::types::InternalResult;
use std::{
    fs::{File, OpenOptions},
    io::{self, ErrorKind, Read, Seek, SeekFrom, Write},
    path::Path,
};

const INDEX_PATH: &'static str = "index.vuoto";
const RECORD_SIZE: usize = 16;
const MAGIC: &[u8; 8] = b"VUOTOIDX";
const VERSION: u32 = 1;
const HEADER_SIZE: usize = MAGIC.len() + 4;

pub struct VaultIndex {
    vaults: Vec<String>,
    file: File,
}

impl VaultIndex {
    /// Open or create index
    pub fn open<P: AsRef<Path>>(dir_path: &P) -> InternalResult<Self> {
        let mut file = Self::read_file(dir_path)?;

        if !Self::check_header(&mut file)? {
            Self::init_file(&mut file)?;
        }

        file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        let mut vaults = Vec::new();
        let mut buf = [0u8; RECORD_SIZE];

        loop {
            let n = file.read(&mut buf)?;

            // EOF
            if n == 0 {
                break;
            }

            // partial record at end, ignore
            if n < RECORD_SIZE {
                break;
            }

            if buf.iter().all(|&b| b == 0) {
                continue;
            }

            let len = buf.iter().position(|&b| b == 0).unwrap_or(RECORD_SIZE);
            let name = std::str::from_utf8(&buf[..len])
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?
                .to_string();

            vaults.push(name);
        }

        // update read/write pointer for future operations
        file.seek(SeekFrom::End(0))?;

        Ok(Self { vaults, file })
    }

    /// Open or create a file handle
    fn read_file<P: AsRef<Path>>(dir_path: &P) -> InternalResult<File> {
        let path = dir_path.as_ref().join(INDEX_PATH);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

        Ok(file)
    }

    /// Validate file metadata
    fn check_header(file: &mut File) -> InternalResult<bool> {
        file.seek(SeekFrom::Start(0))?;
        let mut header = [0u8; HEADER_SIZE];

        match file.read_exact(&mut header) {
            Ok(()) => {
                let (magic_bytes, version_bytes) = header.split_at(MAGIC.len());

                if magic_bytes != MAGIC {
                    return Ok(false);
                }

                let version = u32::from_le_bytes([
                    version_bytes[0],
                    version_bytes[1],
                    version_bytes[2],
                    version_bytes[3],
                ]);

                Ok(version == VERSION)
            }

            Err(err) if err.kind() == ErrorKind::UnexpectedEof => Ok(false),

            Err(err) => Err(err.into()),
        }
    }

    /// Init or re-init index file and write header
    fn init_file(file: &mut File) -> InternalResult<()> {
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;

        // write metadata
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;

        file.flush()?;
        file.sync_data()?;

        // after header, file length == HEADER_SIZE
        file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        Ok(())
    }

    /// compute on-disk offset for a new slot
    fn calculate_offset_for_slot(slot: u64) -> u64 {
        (HEADER_SIZE as u64) + slot * (RECORD_SIZE as u64)
    }

    /// List vault names
    pub fn vaults(&self) -> &[String] {
        &self.vaults
    }

    /// Add a new valut (avoids duplicates)
    pub fn add(&mut self, name: &str) -> InternalResult<()> {
        if name.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "name must be non-empty").into());
        }

        if name.as_bytes().len() > RECORD_SIZE {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("name byte-length must be <= {} bytes", RECORD_SIZE),
            )
            .into());
        }

        if name.as_bytes().iter().any(|&b| b == 0) {
            return Err(io::Error::new(ErrorKind::InvalidInput, "name cannot contain NUL").into());
        }

        // already present
        if self.vaults.iter().any(|v| v == name) {
            return Ok(());
        }

        // prepare encodings (padded w/ zeros)
        let mut record = [0u8; RECORD_SIZE];
        let bytes = name.as_bytes();

        record[..bytes.len()].copy_from_slice(bytes);

        self.file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        let mut buf = [0u8; RECORD_SIZE];
        let mut idx: u64 = 0;
        let mut slot_idx: Option<u64> = None;

        loop {
            match self.file.read_exact(&mut buf) {
                Ok(()) => {
                    if buf.iter().all(|&b| b == 0) {
                        slot_idx = Some(idx);
                        break;
                    }

                    idx += 1;
                }

                Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,

                Err(err) => return Err(err.into()),
            }
        }

        match slot_idx {
            Some(slot) => {
                let pos = Self::calculate_offset_for_slot(slot);
                self.file.seek(SeekFrom::Start(pos))?;
            }

            None => {
                self.file.seek(SeekFrom::End(0))?;
            }
        }

        self.file.write_all(&record)?;
        self.file.flush()?;
        self.file.sync_data()?;

        // Reset file pointer to end for future operations
        self.file.seek(SeekFrom::End(0))?;

        self.vaults.push(name.to_string());

        Ok(())
    }

    /// Delete valut name
    pub fn remove(&mut self, name: &str) -> InternalResult<bool> {
        if !self.vaults.iter().any(|v| v == name) {
            return Ok(false);
        }

        // find the record (first match)
        self.file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        let mut buf = [0u8; RECORD_SIZE];
        let mut idx: u64 = 0;
        let mut found = false;

        while let Ok(()) = self.file.read_exact(&mut buf) {
            if buf.iter().all(|&b| b == 0) {
                idx += 1;
                continue;
            }

            let len = buf.iter().position(|&b| b == 0).unwrap_or(RECORD_SIZE);
            let rec_name = std::str::from_utf8(&buf[..len])
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            if rec_name == name {
                found = true;
                break;
            }

            idx += 1;
        }

        // Always remove from in-memory vector if present
        if let Some(pos) = self.vaults.iter().position(|v| v == name) {
            self.vaults.remove(pos);
        }

        if !found {
            // Not found on disk, but was in memory - still return true since we did remove it
            return Ok(true);
        }

        let pos = Self::calculate_offset_for_slot(idx);
        self.file.seek(SeekFrom::Start(pos))?;

        let zeros = [0u8; RECORD_SIZE];
        self.file.write_all(&zeros)?;

        self.file.flush()?;
        self.file.sync_data()?;

        // Reset file pointer to end for future operations
        self.file.seek(SeekFrom::End(0))?;

        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_temp_dir() -> TempDir {
        tempfile::tempdir().expect("Failed to create temp directory")
    }

    #[test]
    fn test_create_new_index() {
        let temp_dir = setup_temp_dir();
        let index = VaultIndex::open(&temp_dir.path()).unwrap();

        assert_eq!(index.vaults().len(), 0);

        // Verify file exists and has correct header
        let index_path = temp_dir.path().join(INDEX_PATH);
        assert!(index_path.exists());

        let metadata = fs::metadata(&index_path).unwrap();
        assert_eq!(metadata.len(), HEADER_SIZE as u64);
    }

    #[test]
    fn test_open_existing_valid_index() {
        let temp_dir = setup_temp_dir();

        // Create and close first index
        {
            let mut index = VaultIndex::open(&temp_dir.path()).unwrap();
            index.add("test_vault").unwrap();
            assert_eq!(index.vaults().len(), 1);
        }

        // Open existing index
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 1);
        assert_eq!(index.vaults()[0], "test_vault");
    }

    #[test]
    fn test_add_vault_basic() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        index.add("vault1").unwrap();
        assert_eq!(index.vaults().len(), 1);
        assert_eq!(index.vaults()[0], "vault1");

        index.add("vault2").unwrap();
        assert_eq!(index.vaults().len(), 2);
        assert!(index.vaults().contains(&"vault1".to_string()));
        assert!(index.vaults().contains(&"vault2".to_string()));
    }

    #[test]
    fn test_add_duplicate_vault() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        index.add("vault1").unwrap();
        index.add("vault1").unwrap(); // Should not add duplicate

        assert_eq!(index.vaults().len(), 1);
        assert_eq!(index.vaults()[0], "vault1");
    }

    #[test]
    fn test_add_empty_name() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        let result = index.add("");
        assert!(result.is_err());
        assert_eq!(index.vaults().len(), 0);
    }

    #[test]
    fn test_add_name_too_long() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        let long_name = "a".repeat(RECORD_SIZE + 1);
        let result = index.add(&long_name);
        assert!(result.is_err());
        assert_eq!(index.vaults().len(), 0);
    }

    #[test]
    fn test_add_name_max_length() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        let max_name = "a".repeat(RECORD_SIZE);
        index.add(&max_name).unwrap();
        assert_eq!(index.vaults().len(), 1);
        assert_eq!(index.vaults()[0], max_name);
    }

    #[test]
    fn test_add_name_with_null_byte() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        let result = index.add("vault\0name");
        assert!(result.is_err());
        assert_eq!(index.vaults().len(), 0);
    }

    #[test]
    fn test_remove_existing_vault() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        index.add("vault1").unwrap();
        index.add("vault2").unwrap();
        assert_eq!(index.vaults().len(), 2);

        let removed = index.remove("vault1").unwrap();
        assert!(removed);
        assert_eq!(index.vaults().len(), 1);
        assert_eq!(index.vaults()[0], "vault2");
    }

    #[test]
    fn test_remove_nonexistent_vault() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        index.add("vault1").unwrap();

        let removed = index.remove("nonexistent").unwrap();
        assert!(!removed);
        assert_eq!(index.vaults().len(), 1);
        assert_eq!(index.vaults()[0], "vault1");
    }

    #[test]
    fn test_persistence_after_operations() {
        let temp_dir = setup_temp_dir();

        // Perform operations in first instance
        {
            let mut index = VaultIndex::open(&temp_dir.path()).unwrap();
            index.add("vault1").unwrap();
            index.add("vault2").unwrap();
            index.add("vault3").unwrap();
            index.remove("vault2").unwrap();
        }

        // Open new instance and verify persistence
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 2);
        assert!(index.vaults().contains(&"vault1".to_string()));
        assert!(index.vaults().contains(&"vault3".to_string()));
        assert!(!index.vaults().contains(&"vault2".to_string()));
    }

    #[test]
    fn test_slot_reuse_after_removal() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        // Add three vaults
        index.add("vault1").unwrap();
        index.add("vault2").unwrap();
        index.add("vault3").unwrap();

        // Remove the middle one
        index.remove("vault2").unwrap();

        // Add a new vault (should reuse the slot)
        index.add("vault4").unwrap();

        // Verify state
        assert_eq!(index.vaults().len(), 3);
        assert!(index.vaults().contains(&"vault1".to_string()));
        assert!(index.vaults().contains(&"vault3".to_string()));
        assert!(index.vaults().contains(&"vault4".to_string()));
        assert!(!index.vaults().contains(&"vault2".to_string()));

        // Verify persistence
        drop(index);
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 3);
        assert!(index.vaults().contains(&"vault1".to_string()));
        assert!(index.vaults().contains(&"vault3".to_string()));
        assert!(index.vaults().contains(&"vault4".to_string()));
    }

    #[test]
    fn test_unicode_vault_names() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        let unicode_names = vec!["café", "数据库", "🔐vault"];

        for name in &unicode_names {
            if name.as_bytes().len() <= RECORD_SIZE {
                index.add(name).unwrap();
            }
        }

        // Verify persistence with unicode names
        drop(index);
        let index = VaultIndex::open(&temp_dir.path()).unwrap();

        for name in &unicode_names {
            if name.as_bytes().len() <= RECORD_SIZE {
                assert!(index.vaults().contains(&name.to_string()));
            }
        }
    }

    #[test]
    fn test_many_operations() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        // Add many vaults
        for i in 0..100 {
            index.add(&format!("vault_{:03}", i)).unwrap();
        }
        assert_eq!(index.vaults().len(), 100);

        // Remove every other vault
        for i in (0..100).step_by(2) {
            index.remove(&format!("vault_{:03}", i)).unwrap();
        }
        assert_eq!(index.vaults().len(), 50);

        // Add some back
        for i in 0..10 {
            index.add(&format!("new_vault_{}", i)).unwrap();
        }
        assert_eq!(index.vaults().len(), 60);

        // Verify persistence
        drop(index);
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 60);
    }

    #[test]
    fn test_invalid_header_magic() {
        let temp_dir = setup_temp_dir();
        let index_path = temp_dir.path().join(INDEX_PATH);

        // Create file with invalid magic
        let mut file = File::create(&index_path).unwrap();
        file.write_all(b"INVALID!").unwrap();
        file.write_all(&VERSION.to_le_bytes()).unwrap();
        file.flush().unwrap();
        drop(file);

        // Should reinitialize the file
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 0);

        // Verify the file was reinitialized correctly
        let mut file = File::open(&index_path).unwrap();
        let mut header = [0u8; HEADER_SIZE];
        file.read_exact(&mut header).unwrap();

        let (magic_bytes, version_bytes) = header.split_at(MAGIC.len());
        assert_eq!(magic_bytes, MAGIC);
        let version = u32::from_le_bytes([
            version_bytes[0],
            version_bytes[1],
            version_bytes[2],
            version_bytes[3],
        ]);
        assert_eq!(version, VERSION);
    }

    #[test]
    fn test_invalid_header_version() {
        let temp_dir = setup_temp_dir();
        let index_path = temp_dir.path().join(INDEX_PATH);

        // Create file with invalid version
        let mut file = File::create(&index_path).unwrap();
        file.write_all(MAGIC).unwrap();
        file.write_all(&999u32.to_le_bytes()).unwrap(); // Wrong version
        file.flush().unwrap();
        drop(file);

        // Should reinitialize the file
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 0);
    }

    #[test]
    fn test_partial_record_at_end() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();
        index.add("test").unwrap();
        drop(index);

        // Manually append partial record to file
        let index_path = temp_dir.path().join(INDEX_PATH);
        let mut file = OpenOptions::new().append(true).open(&index_path).unwrap();
        file.write_all(&[1, 2, 3, 4, 5]).unwrap(); // Partial record (less than RECORD_SIZE)
        drop(file);

        // Should ignore the partial record
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 1);
        assert_eq!(index.vaults()[0], "test");
    }

    #[test]
    fn test_calculate_offset_for_slot() {
        assert_eq!(VaultIndex::calculate_offset_for_slot(0), HEADER_SIZE as u64);
        assert_eq!(
            VaultIndex::calculate_offset_for_slot(1),
            HEADER_SIZE as u64 + RECORD_SIZE as u64
        );
        assert_eq!(
            VaultIndex::calculate_offset_for_slot(5),
            HEADER_SIZE as u64 + 5 * RECORD_SIZE as u64
        );
    }

    #[test]
    fn test_empty_file() {
        let temp_dir = setup_temp_dir();
        let index_path = temp_dir.path().join(INDEX_PATH);

        // Create empty file
        File::create(&index_path).unwrap();

        // Should initialize properly
        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 0);
    }

    #[test]
    fn test_file_with_only_header() {
        let temp_dir = setup_temp_dir();
        let index_path = temp_dir.path().join(INDEX_PATH);

        // Create file with only valid header
        let mut file = File::create(&index_path).unwrap();
        file.write_all(MAGIC).unwrap();
        file.write_all(&VERSION.to_le_bytes()).unwrap();
        drop(file);

        let index = VaultIndex::open(&temp_dir.path()).unwrap();
        assert_eq!(index.vaults().len(), 0);
    }

    #[test]
    fn test_add_and_remove_same_name_multiple_times() {
        let temp_dir = setup_temp_dir();
        let mut index = VaultIndex::open(&temp_dir.path()).unwrap();

        for _ in 0..5 {
            index.add("test_vault").unwrap();
            assert_eq!(index.vaults().len(), 1);

            let removed = index.remove("test_vault").unwrap();
            assert!(removed);
            assert_eq!(index.vaults().len(), 0);
        }
    }
}
