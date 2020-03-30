use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Response};
use web_sys::console;
//use js_sys::Uint8Array;
use pdf_form_ids::{Form, FieldState};

// Called when the wasm module is instantiated
#[wasm_bindgen(start)]
pub fn main() -> Result<(), JsValue> {
    console::log_1(&"START".into());
    //run();
    console::log_1(&"END".into());
    Ok(())
}

#[wasm_bindgen]
pub async fn load_data() -> Result<(), JsValue> {
    console::log_1(&"AAA".into());

    // Use `web_sys`'s global `window` function to get a handle on the global
    // window object.
    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("should have a document on window");
    let body = document.body().expect("document should have a body");

    let window = web_sys::window().unwrap();
    let pr = window.fetch_with_str("http://localhost:8000/sample.pdf");

    console::log_1(&"ONE".into());

    let resp_value = JsFuture::from(pr).await?;

    // `resp_value` is a `Response` object.
    assert!(resp_value.is_instance_of::<Response>());
    let resp: Response = resp_value.dyn_into().unwrap();

    // Convert this other `Promise` into a rust `Future`.

    console::log_1(&"TWO".into());

    let buf_val = JsFuture::from(resp.array_buffer().unwrap()).await?;
    assert!(buf_val.is_instance_of::<js_sys::ArrayBuffer>());

    let typebuf: js_sys::Uint8Array = js_sys::Uint8Array::new(&buf_val);
    let resp_vec = typebuf.to_vec();

    console::log_1(&"THREE".into());

    let mut resp_body = vec![0; typebuf.length() as usize];
    typebuf.copy_to(&mut resp_body[..]);

    let mut form = Form::load_from(&resp_body[..]).unwrap();
//
//    console::log_1(&"THREE".into());
//
//    // Manufacture the element we're gonna append
//    //let body_str: String = resp_body.into_iter().map(|i| i.to_string()).collect::<String>();
//
//    let val = document.create_element("p")?;
//    val.set_inner_html(resp_vec.as_slice().to_hex());
//    body.append_child(&val)?;

    //console::log_1(&body_str.into());

    Ok(())
}
