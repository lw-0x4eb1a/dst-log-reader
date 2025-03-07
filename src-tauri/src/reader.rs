use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use zip::ZipArchive;
use zip::read::ZipFileSeek;
use rental::rental;

use crate::ds_log::LogPath;

// This macro allows us to define a struct that contains self referential fields.
// see https://docs.rs/rental/0.5.6/rental/
rental! {
    pub mod zip_reader {
        use super::*;
    
        #[rental_mut]
        pub struct ZipReaderImpl {
            archive: Box<ZipArchive<File>>,
            file: ZipFileSeek<'archive, File>,
        }
    }
}

#[allow(clippy::large_enum_variant)]
pub enum LogReader {
    ForFile(File),
    ForZip(zip_reader::ZipReaderImpl),
}

impl LogReader {
    pub fn new(path: &LogPath) -> std::io::Result<Self> {
        if !path.is_zip() {
            let file = File::open(path.get_path())?;
            Ok(LogReader::ForFile(file))
        } else {
            let archive = ZipArchive::new(File::open(path.get_path())?)?;
            // TODO: remove this unwrap
            let inner = zip_reader::ZipReaderImpl::new(Box::new(archive), |a| a.by_name_seek(&path.get_name()).unwrap());
            Ok(LogReader::ForZip(inner))
        }
    }
}

impl Read for LogReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            LogReader::ForFile(f) => f.read(buf),
            LogReader::ForZip(f) => f.rent_mut(|file| file.read(buf)),
        }
    }
}

impl Seek for LogReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            LogReader::ForFile(f) => f.seek(pos),
            LogReader::ForZip(f) => f.rent_mut(|file| file.seek(pos)),
        }
    }
}