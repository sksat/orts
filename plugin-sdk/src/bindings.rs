#![allow(warnings)]

wit_bindgen::generate!({
    world: "plugin",
    path: "wit/v0",
    pub_export_macro: true,
});
