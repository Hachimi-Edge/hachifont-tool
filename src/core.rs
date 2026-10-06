use anyhow::{Result, anyhow, bail};
use rabex::{
	files::{
		SerializedFile,
		bundlefile::{BundleFileBuilder, BundleFileReader, CompressionType, ExtractionConfig},
		serializedfile::{build_common_offset_map, builder::SerializedFileBuilder},
	},
	objects::ClassId,
	tpk::TpkTypeTreeBlob,
	typetree::typetree_cache::TypeTreeCache,
};
use std::{
	borrow::Cow,
	io::{Cursor, Read, Write},
};

#[derive(Clone)]
#[allow(non_snake_case)]
struct ContainerField(Vec<(String, serde_json::Value)>);

impl<'de> serde::Deserialize<'de> for ContainerField {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		deserializer.deserialize_map(MapVisitor).map(ContainerField)
	}
}

struct MapVisitor;

impl<'de> serde::de::Visitor<'de> for MapVisitor {
	type Value = Vec<(String, serde_json::Value)>;

	fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "a map")
	}

	fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
	where
		A: serde::de::MapAccess<'de>,
	{
		let mut out = Vec::new();
		while let Some(pair) = access.next_entry::<String, serde_json::Value>()? {
			out.push(pair);
		}
		Ok(out)
	}
}

impl serde::Serialize for ContainerField {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use serde::ser::SerializeMap;
		let mut map = serializer.serialize_map(Some(self.0.len()))?;
		for (key, value) in &self.0 {
			map.serialize_entry(key, value)?;
		}
		map.end()
	}
}

#[derive(serde::Serialize, serde::Deserialize)]
#[allow(non_snake_case)]
struct AssetBundleObj {
	m_Name: String,
	m_PreloadTable: Vec<serde_json::Value>,
	m_Container: ContainerField,
	m_MainAsset: serde_json::Value,
	m_RuntimeCompatibility: u32,
	m_AssetBundleName: String,
	m_Dependencies: Vec<String>,
	m_IsStreamedSceneAssetBundle: bool,
	m_ExplicitDataLayout: i32,
	m_PathFlags: i32,
	m_SceneHashes: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[allow(non_snake_case)]
struct FontObj {
	m_Name: serde_json::Value,
	m_LineSpacing: serde_json::Value,
	m_DefaultMaterial: serde_json::Value,
	m_FontSize: serde_json::Value,
	m_Texture: serde_json::Value,
	m_AsciiStartOffset: serde_json::Value,
	m_Tracking: serde_json::Value,
	m_CharacterSpacing: serde_json::Value,
	m_CharacterPadding: serde_json::Value,
	m_ConvertCase: serde_json::Value,
	m_CharacterRects: serde_json::Value,
	m_KerningValues: serde_json::Value,
	m_PixelScale: serde_json::Value,
	m_FontData: Vec<char>,
	m_Ascent: serde_json::Value,
	m_Descent: serde_json::Value,
	m_DefaultStyle: serde_json::Value,
	m_FontNames: serde_json::Value,
	m_FallbackFonts: serde_json::Value,
	m_FontRenderingMode: serde_json::Value,
	m_UseLegacyBoundsCalculation: serde_json::Value,
	m_ShouldRoundAdvanceValue: serde_json::Value,
}

pub fn inspect(bundle_bytes: &[u8], font_path: &str) -> Result<serde_json::Value> {
	let tpk = tpk();
	let config = ExtractionConfig::default().assume_recent_unity();
	let mut bundle = BundleFileReader::from_reader(Cursor::new(bundle_bytes), &config)?;

	while let Some(mut file) = bundle.next_serialized() {
		let data = file.read()?;
		let mut reader = Cursor::new(data);
		let serialized = SerializedFile::from_reader(&mut reader)?;
		for object in serialized.objects() {
			if object.m_ClassID != ClassId::Font {
				continue;
			}

			let value: serde_json::Value = serialized.read(object, &tpk, &mut reader)?;
			let name = value["m_Name"].as_str().unwrap_or_default();
			if name_matches(name, font_path) {
				let font_data_len = value["m_FontData"]
					.as_array()
					.map(|items| items.len())
					.unwrap_or(0);
				return Ok(serde_json::json!({
					"name": name,
					"fontDataLength": font_data_len,
					"filePath": file.path,
				}));
			}
		}
	}

	bail!("font asset not found for path '{font_path}'")
}

pub fn replace(
	bundle_bytes: &[u8],
	old_font_path: &str,
	new_font_file_name: &str,
	new_font_data: &[u8],
) -> Result<Vec<u8>> {
	let tpk = tpk();
	let config = ExtractionConfig::default().assume_recent_unity();
	let comp = detect_compression(bundle_bytes, &config)?;
	let mut version = None;
	let mut replaced_files: Vec<(String, Vec<u8>)> = Vec::new();
	let mut bundle = BundleFileReader::from_reader(Cursor::new(bundle_bytes), &config)?;
	while let Some(mut file) = bundle.next_serialized() {
		let data = file.read()?;
		let mut reader = Cursor::new(data);
		let serialized = SerializedFile::from_reader(&mut reader)?;
		let unity_version = serialized
			.m_UnityVersion
			.clone()
			.ok_or_else(|| anyhow!("missing unity version"))?;
		version.get_or_insert_with(|| unity_version.clone());

		let tpk_raw = TpkTypeTreeBlob::embedded();
		let common_offset_map = build_common_offset_map(&tpk_raw, &unity_version);
		let mut builder = SerializedFileBuilder::from_serialized(
			&unity_version,
			&serialized,
			data,
			&tpk,
			&common_offset_map,
			serialized.objects().cloned(),
		);

		for object in serialized.objects() {
			let v = match object.m_ClassID {
				ClassId::Font => rewrite_font(
					&serialized,
					object,
					&tpk,
					&mut reader,
					old_font_path,
					new_font_file_name,
					new_font_data,
				)?,
				ClassId::AssetBundle => rewrite_asset_bundle(
					&serialized,
					object,
					&tpk,
					&mut reader,
					old_font_path,
					new_font_file_name,
				)?,
				_ => None,
			};

			if let Some(bytes) = v && let Some(entry) = builder.objects.get_mut(&object.m_PathID) {
				entry.1 = Cow::Owned(bytes);
			}
		}

		let mut out = Vec::new();
		builder.write(&mut Cursor::new(&mut out))?;
		replaced_files.push((file.path.to_string(), out));
	}

	let version = version.ok_or_else(|| anyhow!("no serialized file"))?;
	let mut out_builder = BundleFileBuilder::unityfs(7, &version);
	let bundle = BundleFileReader::from_reader(Cursor::new(bundle_bytes), &config)?;
	for entry in bundle.files().iter() {
		let data = bundle.read_at_entry(entry)?;
		let bytes = replaced_files
			.iter()
			.find(|(path, _)| path == &entry.path)
			.map(|(_, out)| out.clone())
			.unwrap_or(data);
		out_builder.add_file_with_flags(&entry.path, Cursor::new(bytes), entry.flags)?;
	}

	let mut out = Vec::new();
	out_builder.write(&mut Cursor::new(&mut out), comp)?;
	Ok(out)
}

fn detect_compression(bundle_bytes: &[u8], config: &ExtractionConfig) -> Result<CompressionType> {
	let bundle = BundleFileReader::from_reader(Cursor::new(bundle_bytes), config)?;
	Ok(bundle
		.blocks()
		.first()
		.and_then(|block| CompressionType::try_from(block.flags & 0x3F).ok())
		.unwrap_or(CompressionType::None))
}

fn rewrite_font(
	serialized: &SerializedFile,
	object: &rabex::files::serializedfile::ObjectInfo,
	tpk: &TypeTreeCache<TpkTypeTreeBlob>,
	reader: &mut Cursor<&[u8]>,
	old_font_path: &str,
	new_font_file_name: &str,
	new_font_data: &[u8],
) -> Result<Option<Vec<u8>>> {
	let mut font: FontObj = serialized.read(object, tpk, reader)?;
	let name = font.m_Name.as_str().unwrap_or_default();

	if !name_matches(name, old_font_path) {
		return Ok(None);
	}

	font.m_Name = new_font_file_name
		.rsplit_once('.')
		.map(|(stem, _)| stem)
		.unwrap_or(new_font_file_name)
		.into();
	font.m_FontData = new_font_data
		.iter()
		.map(|&b| char::from_u32(b as u32).unwrap_or('\u{0}'))
		.collect();

	let typetree = serialized.get_typetree_for(object, tpk)?;
	rabex::serde_typetree::to_vec_endianed(&font, &typetree, serialized.m_Header.m_Endianess)
		.map(Some)
		.map_err(Into::into)
}

fn rewrite_asset_bundle(
	serialized: &SerializedFile,
	object: &rabex::files::serializedfile::ObjectInfo,
	tpk: &TypeTreeCache<TpkTypeTreeBlob>,
	reader: &mut Cursor<&[u8]>,
	old_font_path: &str,
	new_font_file_name: &str,
) -> Result<Option<Vec<u8>>> {
	let mut bundle: AssetBundleObj = serialized.read(object, tpk, reader)?;
	let mut changed = false;
	for (key, _) in bundle.m_Container.0.iter_mut() {
		if let Some(renamed) = rename_key(key, old_font_path, new_font_file_name) {
			*key = renamed;
			changed = true;
		}
	}

	if !changed {
		return Ok(None);
	}

	let typetree = serialized.get_typetree_for(object, tpk)?;
	rabex::serde_typetree::to_vec_endianed(&bundle, &typetree, serialized.m_Header.m_Endianess)
		.map(Some)
		.map_err(Into::into)
}

const MAX_BUNDLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FONT_PATH_BYTES: usize = 4 * 1024;

pub fn parse_hachifont(buf: &[u8]) -> Result<(Vec<u8>, Vec<u8>, String)> {
	let mut zip = zip::ZipArchive::new(Cursor::new(buf))?;
	let mut android = Vec::new();
	let mut win = Vec::new();
	let mut font_path = String::new();
	let mut android_entry = zip.by_name("includes_android")?;

	if android_entry.size() > MAX_BUNDLE_BYTES {
		bail!("includes_android exceeds {MAX_BUNDLE_BYTES} bytes");
	}

	android_entry.read_to_end(&mut android)?;
	drop(android_entry);
	let mut win_entry = zip.by_name("includes_win")?;
	if win_entry.size() > MAX_BUNDLE_BYTES {
		bail!("includes_win exceeds {MAX_BUNDLE_BYTES} bytes");
	}
	win_entry.read_to_end(&mut win)?;
	drop(win_entry);

	let mut font_path_entry = zip.by_name("font_path.txt")?;
	if font_path_entry.size() > MAX_FONT_PATH_BYTES as u64 {
		bail!("font_path.txt exceeds {MAX_FONT_PATH_BYTES} bytes");
	}

	font_path_entry.read_to_string(&mut font_path)?;
	Ok((android, win, font_path.trim().to_string()))
}

pub fn build_hachifont(android: &[u8], win: &[u8], font_path: &str) -> Result<Vec<u8>> {
	if android.len() as u64 > MAX_BUNDLE_BYTES || win.len() as u64 > MAX_BUNDLE_BYTES {
		bail!("bundle exceeds {MAX_BUNDLE_BYTES} bytes");
	}

	if font_path.len() > MAX_FONT_PATH_BYTES {
		bail!("font_path exceeds {MAX_FONT_PATH_BYTES} bytes");
	}

	let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
	let options = zip::write::SimpleFileOptions::default()
		.compression_method(zip::CompressionMethod::Deflated);
	zip.start_file("includes_android", options)?;
	zip.write_all(android)?;
	zip.start_file("includes_win", options)?;
	zip.write_all(win)?;
	zip.start_file("font_path.txt", options)?;
	zip.write_all(font_path.as_bytes())?;
	Ok(zip.finish()?.into_inner())
}

fn tpk() -> TypeTreeCache<TpkTypeTreeBlob> {
	TypeTreeCache::new(TpkTypeTreeBlob::embedded())
}

fn stem(name: &str) -> &str {
	name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

fn basename(path: &str) -> &str {
	path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn name_matches(name: &str, font_path: &str) -> bool {
	let base = basename(font_path);
	name.eq_ignore_ascii_case(font_path)
		|| name.eq_ignore_ascii_case(base)
		|| name.eq_ignore_ascii_case(stem(base))
}

fn rename_key(key: &str, old_font_path: &str, new_font_file_name: &str) -> Option<String> {
	let key_base = basename(key);
	let old_base = basename(old_font_path);
	if !(key.eq_ignore_ascii_case(old_font_path)
		|| key_base.eq_ignore_ascii_case(old_base)
		|| stem(key_base).eq_ignore_ascii_case(stem(old_base)))
	{
		return None;
	}
	Some(replace_file_name(key, new_font_file_name).to_lowercase())
}

fn replace_file_name(path: &str, new_file_name: &str) -> String {
	match path.rsplit_once(['/', '\\']) {
		Some((dir, _)) => format!("{dir}/{new_file_name}"),
		None => new_file_name.to_string(),
	}
}