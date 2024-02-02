use binary_parser::*;
use clap::Parser;
use libflate::gzip;
use std::{
	collections::BTreeMap,
	fs::File,
	io::{Cursor, Read, SeekFrom, Write},
	path::{Path, PathBuf},
};

#[derive(Parser)]
struct Args {
	path: String,
}

fn main() {
	let args = Args::parse();
	let path = PathBuf::from(args.path);
	if !path.exists() {
		panic!("Path must exist");
	} else if path.is_dir() {
		let xmd = Xmd::from_files(&path).unwrap();
		xmd.write_file(path.with_extension("xmd"), true).unwrap();
	} else if path.is_file() {
		let xmd = Xmd::from_file(&path).unwrap();
		let folder = format!(
			"{}_{}",
			path.with_extension("").to_str().unwrap(),
			path.extension().unwrap().to_str().unwrap()
		);
		std::fs::create_dir(&folder).unwrap();
		for (id, data) in &xmd.files {
			let mut buf = [0; 4];
			data.take(4).read(&mut buf).unwrap();
			let name = if buf == [b'N', b'D', b'W', b'D'] {
				let mut reader = BinaryParser::from_buf(data.clone());
				reader.seek(SeekFrom::Start(0x10)).unwrap();
				let poly_start = reader.read_u32().unwrap() + 0x30;
				let poly_size = reader.read_u32().unwrap();
				let vert_size = reader.read_u32().unwrap();
				let vert_add_size = reader.read_u32().unwrap();
				let name_pos = poly_start + poly_size + vert_size + vert_add_size;
				reader.seek(SeekFrom::Start(name_pos as u64)).unwrap();
				let name = reader.read_null_string().unwrap();
				let name = name
					.chars()
					.filter(|c| c.is_alphabetic() || c == &'_')
					.collect::<String>();

				format!("{}_{}.nud", name, id)
			} else if buf == [b'N', b'T', b'W', b'D'] {
				format!("{}.nut", id)
			} else {
				format!("{}", id)
			};
			let mut file = File::create(format!("{}/{}", folder, name)).unwrap();
			file.write(data).unwrap();
		}
	} else {
		panic!("What have you done");
	}
}

#[derive(Default, Debug)]
pub struct Xmd {
	pub files: BTreeMap<u32, Vec<u8>>,
}

impl Xmd {
	pub fn from_files<P: Into<PathBuf>>(path: P) -> Option<Self> {
		let path: PathBuf = path.into();
		if !path.is_dir() {
			return None;
		}
		let mut xmd = Self::default();
		for file in std::fs::read_dir(path).ok()? {
			let file = file.ok()?;
			if file.path().is_file() {
				if let Ok(id) = file
					.path()
					.with_extension("")
					.file_name()
					.unwrap()
					.to_str()
					.unwrap()
					.chars()
					.filter(|c| c.is_numeric())
					.collect::<String>()
					.parse::<u32>()
				{
					let data = std::fs::read(file.path()).ok()?;
					xmd.files.insert(id, data);
				}
			}
		}

		Some(xmd)
	}

	pub fn from_file<P: AsRef<Path>>(path: P) -> Option<Self> {
		let mut reader = BinaryParser::from_file(path).ok()?;
		let id = reader.read_buf(3).ok()?;
		if id == [0x1F, 0x8B, 0x08] {
			let mut cursor = Cursor::new(reader.to_buf().ok()?);
			let mut decoder = gzip::Decoder::new(&mut cursor).ok()?;
			let mut data = Vec::new();
			decoder.read_to_end(&mut data).ok()?;
			reader = BinaryParser::from_buf(data);
		}
		reader.seek(SeekFrom::Start(0)).ok()?;
		Self::from_parser(&mut reader)
	}

	pub fn from_parser(reader: &mut BinaryParser) -> Option<Self> {
		let id = reader.read_null_string().ok()?;
		if id != "XMD" {
			return None;
		}
		_ = reader.read_u32().ok()?;
		_ = reader.read_u32().ok()?;
		let count = reader.read_u32().ok()? as usize;

		let offsets = reader.read_u32_array(count as u64).ok()?;
		reader.align_seek(0x10).ok()?;
		let lengths = reader.read_u32_array(count as u64).ok()?;
		reader.align_seek(0x10).ok()?;
		let ids = reader.read_u32_array(count as u64).ok()?;

		let mut i = 0;
		let mut xmd = Self::default();
		while i < count {
			reader.seek(SeekFrom::Start(offsets[i] as u64)).ok()?;
			let data = reader.read_buf(lengths[i] as usize).ok()?;
			xmd.files.insert(ids[i], data);
			i += 1;
		}

		Some(xmd)
	}

	pub fn write_file<P: AsRef<Path>>(&self, path: P, compress: bool) -> Option<()> {
		let mut file = File::create(path).ok()?;
		let parser = self.write_parser(compress)?;
		file.write(&parser.to_buf_const().unwrap()).ok()?;
		Some(())
	}

	pub fn write_parser(&self, compress: bool) -> Option<BinaryParser> {
		let mut writer = BinaryParser::new();
		writer.write_string("XMD\0001\0").ok()?;
		writer.write_u32(3).ok()?;
		writer.write_u32(self.files.len() as u32).ok()?;
		for (_, file) in &self.files {
			let file = file.clone();
			writer
				.write_pointer(move |writer| {
					writer.write_buf(&file)?;
					writer.align_write(0x10)
				})
				.ok()?;
		}

		writer.align_write(0x10).ok()?;
		let lengths = self
			.files
			.iter()
			.map(|(_, file)| file.len() as u32)
			.collect::<Vec<_>>();
		writer.write_u32_array(&lengths).ok()?;
		writer.align_write(0x10).ok()?;
		let ids = self.files.iter().map(|(id, _)| *id).collect::<Vec<_>>();
		writer.write_u32_array(&ids).ok()?;
		writer.align_write(0x10).ok()?;

		writer.finish_writes().ok()?;
		let writer = if compress {
			let buf = writer.to_buf_const().unwrap();
			let mut encoder = gzip::Encoder::new(vec![]).ok()?;
			encoder.write(buf).ok()?;
			let data = encoder.finish().into_result().ok()?;
			BinaryParser::from_buf(data)
		} else {
			writer
		};
		Some(writer)
	}
}
