use anyhow::{Result, anyhow};
use serde_json::{self, Value};

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

pub fn download(id: &str) -> Result<(String, String)> {
    let (name, code) = get_shader_name_and_code(id)?;

    let path = Path::new("./downloaded/");
    if !path.exists() {
        std::fs::create_dir(path)?;
    }
    File::create(path.join(&name))
        .or_else(|err| Err(anyhow!("error creating downloaded shader {:?}", err)))?
        .write_all(code.as_bytes())
        .or_else(|err| Err(anyhow!("error writing downloaded shader {:?}", err)))?;

    Ok((name, code))
}

fn get_shader_name_and_code(mut id: &str) -> Result<(String, String)> {
    let https_url = "https://www.shadertoy.com/view/";
    let http_url = "http://www.shadertoy.com/view/";
    let url = "www.shadertoy.com/view/";

    if id.starts_with(https_url) || id.starts_with(http_url) || id.starts_with(url) {
        id = id.split_at(id.rfind("view/").unwrap() + 5).1;
    }

    let json = serde_json::from_str::<Value>(&get_json_string(id)?)?;

    extract_from_json(&json)
}

fn get_json_string(id: &str) -> Result<String> {
    let client = reqwest::blocking::Client::new();
    use reqwest::header::*;
    let mut headers = HeaderMap::new();
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://www.shadertoy.com/"),
    );
    let mut res = client
        .post("https://www.shadertoy.com/shadertoy/")
        .headers(headers)
        .form(&[("s", format!("{{\"shaders\": [\"{}\"]}}", id))])
        .send()?;

    let mut buf = String::new();

    match res.read_to_string(&mut buf) {
        Ok(_) => {
            if buf == "[]" {
                Err(anyhow!("empty response?"))
            } else {
                Ok(buf)
            }
        }
        Err(err) => Err(err.into()),
    }
}

fn extract_from_json(json: &Value) -> Result<(String, String)> {
    let name = format!(
        "{}.frag",
        json[0]["info"]["name"].as_str().unwrap().replace(' ', "_")
    )
    .to_lowercase();
    let mut code = String::new();

    let shaders = json[0]["renderpass"].as_array().unwrap();

    if shaders.len() > 1 {
        for shader in shaders {
            if shader["name"] == "Image" {
                code = String::from(shader["code"].as_str().unwrap());
            }
        }
    } else {
        code = String::from(shaders[0]["code"].as_str().unwrap());
    }

    Ok((name, code))
}
