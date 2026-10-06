#![forbid(unsafe_code)]

pub mod core;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn parse_hachifont(buf: &[u8]) -> Result<JsValue, JsValue> {
	let (android, win, font_path) =
		core::parse_hachifont(buf).map_err(|e| JsValue::from(e.to_string()))?;
	let out = js_sys::Object::new();
	set(
		&out,
		"android",
		js_sys::Uint8Array::from(android.as_slice()),
	);
	set(&out, "win", js_sys::Uint8Array::from(win.as_slice()));
	set(&out, "fontPath", JsValue::from_str(&font_path));
	Ok(out.into())
}

#[wasm_bindgen]
pub fn build_hachifont(
	android: &[u8],
	win: &[u8],
	font_path: &str,
) -> Result<js_sys::Uint8Array, JsValue> {
	core::build_hachifont(android, win, font_path)
		.map(|b| js_sys::Uint8Array::from(b.as_slice()))
		.map_err(|e| JsValue::from(e.to_string()))
}

#[wasm_bindgen]
pub fn inspect_font(bundle: &[u8], font_path: &str) -> Result<JsValue, JsValue> {
	let value = core::inspect(bundle, font_path).map_err(|e| JsValue::from(e.to_string()))?;
	Ok(JsValue::from_str(&value.to_string()))
}

#[wasm_bindgen]
pub fn replace_font(
	android: &[u8],
	win: &[u8],
	old_font_path: &str,
	new_font_file_name: &str,
	new_font_data: &[u8],
) -> Result<JsValue, JsValue> {
	let android = core::replace(android, old_font_path, new_font_file_name, new_font_data)
		.map_err(|e| JsValue::from(e.to_string()))?;
	let win = core::replace(win, old_font_path, new_font_file_name, new_font_data)
		.map_err(|e| JsValue::from(e.to_string()))?;
	let out = js_sys::Object::new();
	set(
		&out,
		"android",
		js_sys::Uint8Array::from(android.as_slice()),
	);
	set(&out, "win", js_sys::Uint8Array::from(win.as_slice()));
	Ok(out.into())
}

fn set(obj: &js_sys::Object, key: &str, value: impl Into<JsValue>) {
	let _ = js_sys::Reflect::set(obj, &JsValue::from_str(key), &value.into());
}
