#![allow(non_snake_case)]

mod glue;

use core::alloc::Layout;
use core::ffi::c_void;
use core::mem::MaybeUninit;
use glue::Metadata;
use libc;
use roc_std::{RocList, RocStr};
use std::ffi::CStr;
use std::os::raw::c_char;
use std::time::Duration;

extern "C" {
    #[link_name = "roc__mainForHost_1_exposed_generic"]
    fn roc_main(output: *mut u8);

    #[link_name = "roc__mainForHost_size"]
    fn roc_main_size() -> i64;

    #[link_name = "roc__mainForHost_1__Fx_caller"]
    fn call_Fx(flags: *const u8, closure_data: *const u8, output: *mut u8);

    #[allow(dead_code)]
    #[link_name = "roc__mainForHost_1__Fx_size"]
    fn size_Fx() -> i64;

    #[link_name = "roc__mainForHost_1__Fx_result_size"]
    fn size_Fx_result() -> i64;
}

#[no_mangle]
pub unsafe extern "C" fn roc_alloc(size: usize, _alignment: u32) -> *mut c_void {
    libc::malloc(size)
}

#[no_mangle]
pub unsafe extern "C" fn roc_realloc(
    c_ptr: *mut c_void,
    new_size: usize,
    _old_size: usize,
    _alignment: u32,
) -> *mut c_void {
    libc::realloc(c_ptr, new_size)
}

#[no_mangle]
pub unsafe extern "C" fn roc_dealloc(c_ptr: *mut c_void, _alignment: u32) {
    libc::free(c_ptr)
}

#[no_mangle]
pub unsafe extern "C" fn roc_panic(c_ptr: *mut c_void, tag_id: u32) {
    match tag_id {
        0 => {
            let slice = CStr::from_ptr(c_ptr as *const c_char);
            let string = slice.to_str().unwrap();
            eprintln!("Roc hit a panic: {}", string);
            std::process::exit(1);
        }
        _ => todo!(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn roc_memcpy(dst: *mut c_void, src: *mut c_void, n: usize) -> *mut c_void {
    libc::memcpy(dst, src, n)
}

#[no_mangle]
pub unsafe extern "C" fn roc_memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    libc::memset(dst, c, n)
}

#[no_mangle]
pub extern "C" fn rust_main() -> i32 {
    let size = unsafe { roc_main_size() } as usize;
    let layout = Layout::array::<u8>(size).unwrap();

    unsafe {
        // TODO allocate on the stack if it's under a certain size
        let buffer = std::alloc::alloc(layout);

        roc_main(buffer);

        let result = call_the_closure(buffer);

        std::alloc::dealloc(buffer, layout);

        result
    };

    // Exit code
    0
}

unsafe fn call_the_closure(closure_data_ptr: *const u8) -> i64 {
    let size = size_Fx_result() as usize;
    let layout = Layout::array::<u8>(size).unwrap();
    let buffer = std::alloc::alloc(layout) as *mut u8;

    call_Fx(
        // This flags pointer will never get dereferenced
        MaybeUninit::uninit().as_ptr(),
        closure_data_ptr as *const u8,
        buffer as *mut u8,
    );

    std::alloc::dealloc(buffer, layout);

    0
}

#[no_mangle]
pub extern "C" fn roc_fx_getLine() -> RocStr {
    use std::io::{self, BufRead};

    let stdin = io::stdin();
    let line1 = stdin.lock().lines().next().unwrap().unwrap();

    RocStr::from(line1.as_str())
}

#[no_mangle]
pub extern "C" fn roc_fx_putLine(line: &RocStr) {
    let string = line.as_str();
    println!("{}", string);
}

#[no_mangle]
pub extern "C" fn roc_fx_sendRequest(roc_request: &glue::Request) -> glue::Response {
    let mut builder = reqwest::blocking::ClientBuilder::new();

    if roc_request.timeout.discriminant() == glue::discriminant_TimeoutConfig::TimeoutMilliseconds {
        let ms: &u64 = unsafe { roc_request.timeout.as_TimeoutMilliseconds() };
        builder = builder.timeout(Duration::from_millis(*ms));
    }

    let client = match builder.build() {
        Ok(c) => c,
        Err(_) => {
            return glue::Response::NetworkError; // TLS backend cannot be initialized
        }
    };

    let method = match roc_request.method {
        glue::Method::Connect => reqwest::Method::CONNECT,
        glue::Method::Delete => reqwest::Method::DELETE,
        glue::Method::Get => reqwest::Method::GET,
        glue::Method::Head => reqwest::Method::HEAD,
        glue::Method::Options => reqwest::Method::OPTIONS,
        glue::Method::Patch => reqwest::Method::PATCH,
        glue::Method::Post => reqwest::Method::POST,
        glue::Method::Put => reqwest::Method::PUT,
        glue::Method::Trace => reqwest::Method::TRACE,
    };

    let url = roc_request.url.as_str();

    let mut req_builder = client.request(method, url);
    for header in roc_request.headers.iter() {
        let (name, value) = unsafe { header.as_Header() };
        req_builder = req_builder.header(name.as_str(), value.as_str());
    }
    if roc_request.body.discriminant() == glue::discriminant_Body::Body {
        let (mime_type_tag, body_byte_list) = unsafe { roc_request.body.as_Body() };
        let mime_type_str: &RocStr = unsafe { mime_type_tag.as_MimeType() };

        req_builder = req_builder.header("Content-Type", mime_type_str.as_str());
        req_builder = req_builder.body(body_byte_list.as_slice().to_vec());
    }

    let request = match req_builder.build() {
        Ok(req) => req,
        Err(err) => {
            return glue::Response::BadRequest(RocStr::from(err.to_string().as_str()));
        }
    };

    match client.execute(request) {
        Ok(response) => {
            let status = response.status();
            let status_str = status.canonical_reason().unwrap_or_else(|| status.as_str());

            let headers_iter = response.headers().iter().map(|(name, value)| {
                glue::Header::Header(
                    RocStr::from(name.as_str()),
                    RocStr::from(value.to_str().unwrap_or_default()),
                )
            });

            let metadata = Metadata {
                headers: RocList::from_iter(headers_iter),
                statusText: RocStr::from(status_str),
                url: RocStr::from(url),
                statusCode: status.as_u16(),
            };

            let bytes = response.bytes().unwrap_or_default();
            let body: RocList<u8> = RocList::from_iter(bytes.into_iter());

            if status.is_success() {
                glue::Response::GoodStatus(metadata, body)
            } else {
                glue::Response::BadStatus(metadata, body)
            }
        }
        Err(err) => {
            if err.is_timeout() {
                glue::Response::Timeout
            } else if err.is_request() {
                glue::Response::BadRequest(RocStr::from(err.to_string().as_str()))
            } else {
                glue::Response::NetworkError
            }
        }
    }
}
