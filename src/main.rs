extern crate dotenv;

extern crate iron;
extern crate mount;
extern crate router;
extern crate staticfile;

extern crate serde;
extern crate serde_json;

extern crate tempfile;

mod tsc;

use iron::prelude::*;
use iron::status;
use mount::Mount;
use router::Router;
use staticfile::Static;

use iron::method::Method;

use std::collections::HashMap;
use std::env;
use std::io::prelude::*;
use std::process::Child;
use std::sync::{Arc, Mutex};

type RequestData = HashMap<String, String>;

#[derive(Default, Clone)]
struct PXExecState {
    running_process: Arc<Mutex<Option<Child>>>,
}

impl PXExecState {
    fn new() -> PXExecState {
        PXExecState {
            running_process: Arc::new(Mutex::new(None)),
        }
    }

    fn replace_child(&self, c: Child) {
        let mut cur_child = self.running_process.lock().unwrap();
        *cur_child = Some(c);
    }

    fn take_child(&self) -> Option<Child> {
        // This single line replaces running_process with None and returns
        //   whatever it was before
        self.running_process.lock().unwrap().take()
    }
}

fn handle_request(body: RequestData, s: &PXExecState) -> Result<(), String> {
    if let Some(main_text) = body.get("main.ts") {
        let assembled = tsc::assemble_main(main_text);
        let mut bytes: &[u8] = assembled.as_bytes();
        return match tsc::compile_with_tempfile(&mut bytes) {
            Ok(path) => {
                s.replace_child(tsc::execute(&path, s.take_child()));
                Ok(())
            }
            Err(msg) => Err(msg),
        };
    }
    Err(String::from("No main.ts file was included in the bundle."))
}

// This application is simple enough that I think we won't need router
// middleware or similar. That option remains open, but for now I'll just return
// 404 messages with a body that simply says "NOT FOUND" in case the message
// is not a POST message or is not to simply "/save"
//
// In case the message is invalid, I'll send a generic 400 with the text
// "YOUR FAULT" followed by a new line and any relevant error message, if one
// exists.
//
/// Dispatch HTTP handler.
fn validate_request(req: &mut Request, s: &PXExecState) -> IronResult<Response> {
    println!("Message received: {:?}", req);
    let mut data = String::new();
    req.body.read_to_string(&mut data).unwrap();
    match req.method {
        Method::Post => {
            let deserialized = serde_json::from_str(&data);
            match deserialized {
                Ok(obj) => match handle_request(obj, s) {
                    Ok(_) => Ok(Response::with((status::Ok, "ACCEPTED"))),
                    Err(msg) => Ok(Response::with((
                        status::InternalServerError,
                        format!("MY FAULT\n{}", msg),
                    ))),
                },
                Err(msg) => Ok(Response::with((
                    status::BadRequest,
                    format!("YOUR FAULT\n{:?}", msg),
                ))),
            }
        }
        _ => Ok(Response::with((status::NotFound, "NOT FOUND"))),
    }
}

fn kill(_req: &mut Request, s: &PXExecState) -> IronResult<Response> {
    println!("Killing child process");
    if let Some(mut child) = s.take_child() {
        match child.kill() {
            Ok(_) => Ok(Response::with(status::Ok)),
            Err(_) => Ok(Response::with(status::InternalServerError)),
        }
    } else {
        Ok(Response::with(status::Ok))
    }
}

fn main() {
    dotenv::dotenv().ok();
    println!("Starting server... ");
    let state = PXExecState::new();
    let state_kill = state.clone();
    let mut router = Router::new();

    router
        .post(
            "/save",
            move |req: &mut Request| validate_request(req, &state),
            "save",
        )
        .post(
            "/kill",
            move |req: &mut Request| kill(req, &state_kill),
            "kill",
        );

    let mut mount = Mount::new();

    // rustfmt insists that this block should be laid out this way... I'm sorry
    mount
        .mount(
            "/editor",
            // If PXEXEC_OVERRIDE_EDITOR_PATH is set for debugging purposes, mount that, otherwise look in ./editor/
            Static::new(
                env::var("PXEXEC_OVERRIDE_EDITOR_PATH").unwrap_or(String::from("./editor/")),
            ),
        )
        .mount("/api", router);

    println!("Done!");

    // If PXEXEC_HOST is set for debugging purposes, bind that, otherwise bind 0.0.0.0:80
    Iron::new(mount)
        .http(env::var("PXEXEC_HOST").unwrap_or(String::from("0.0.0.0:80")))
        .unwrap();
}
