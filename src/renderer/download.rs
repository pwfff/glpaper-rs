use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::output_surface::ArgValues;

pub async fn download(av: &mut ArgValues) -> Result<(String, String)> {
    let (name, code) = get_shader_name_and_code(av).await?;

    write_file(&make_path(&name, &format!("{name}.frag"))?, code.as_bytes())?;

    let path = Path::new("./downloaded/");
    if !path.exists() {
        std::fs::create_dir(path)?;
    }

    Ok((name, code))
}

fn make_path(name: &String, fname: &String) -> Result<PathBuf> {
    let path = &Path::new("./downloaded/").to_path_buf();
    if !path.exists() {
        std::fs::create_dir(path)?;
    }
    let path = &path.join(name);
    if !path.exists() {
        std::fs::create_dir(path)?;
    }
    let path = &path.join(fname);
    Ok(path.clone())
}

fn write_file(path: &PathBuf, val: &[u8]) -> Result<()> {
    File::create(path)
        .or_else(|err| Err(anyhow!("error creating downloaded shader {:?}", err)))?
        .write_all(val)
        .or_else(|err| Err(anyhow!("error writing downloaded shader {:?}", err)))?;

    Ok(())
}

fn addr_mode(s: &String) -> wgpu::AddressMode {
    match s.as_str() {
        "repeat" => wgpu::AddressMode::Repeat,
        "clamp" => wgpu::AddressMode::ClampToEdge,
        "mirror" => wgpu::AddressMode::MirrorRepeat,
        "border" => wgpu::AddressMode::ClampToBorder,
        _ => wgpu::AddressMode::Repeat,
    }
}

async fn get_shader_name_and_code(av: &mut ArgValues) -> Result<(String, String)> {
    let https_url = "https://www.shadertoy.com/view/";
    let http_url = "http://www.shadertoy.com/view/";
    let url = "www.shadertoy.com/view/";

    let mut id = av
        .getid
        .clone()
        .ok_or(anyhow!("cant download with no id"))?;

    if id.starts_with(https_url) || id.starts_with(http_url) || id.starts_with(url) {
        id = id.split_at(id.rfind("view/").unwrap() + 5).1.to_string();
    }

    let json = serde_json::from_str::<Vec<Response>>(&get_json_string(&id).await?)?;
    let first = &json[0];

    let name = format!("{}", first.info.name.replace(' ', "_")).to_lowercase();

    let shader = &first.renderpass[0];

    for input in shader.inputs.iter() {
        println!("getting {}", input.filepath);

        let basename = Path::new(&input.filepath)
            .file_name()
            .ok_or(anyhow!("couldnt get base name"))?
            .to_str()
            .ok_or(anyhow!("wtf is an osstring"))?
            .to_string();

        let path = make_path(&name, &basename)?;

        if !path.exists() {
            let img_bytes = reqwest::get(format!("https://shadertoy.com{}", &input.filepath))
                .await?
                .bytes()
                .await?;

            write_file(&path, &img_bytes)?;
        }

        match input.channel {
            0 => {
                av.texture0path = Some(path.into_os_string().into_string().unwrap());
                av.wrap0 = addr_mode(&input.sampler.wrap);
                av.filter0 = if input.sampler.filter == "mipmap" {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                };
            }
            1 => {
                av.texture1path = Some(path.into_os_string().into_string().unwrap());
                av.wrap1 = addr_mode(&input.sampler.wrap);
                av.filter1 = if input.sampler.filter == "mipmap" {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                };
            }
            2 => {
                av.texture2path = Some(path.into_os_string().into_string().unwrap());
                av.wrap2 = addr_mode(&input.sampler.wrap);
                av.filter2 = if input.sampler.filter == "mipmap" {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                };
            }
            3 => {
                av.texture3path = Some(path.into_os_string().into_string().unwrap());
                av.wrap3 = addr_mode(&input.sampler.wrap);
                av.filter3 = if input.sampler.filter == "mipmap" {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                };
            }
            _ => {}
        }
    }

    Ok((name, shader.code.clone()))
}

async fn get_json_string(id: &str) -> Result<String> {
    let client = reqwest::Client::new();
    use reqwest::header::*;
    let mut headers = HeaderMap::new();
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://www.shadertoy.com/"),
    );
    let res = client
        .post("https://www.shadertoy.com/shadertoy/")
        .headers(headers)
        .form(&[("s", format!("{{\"shaders\": [\"{}\"]}}", id))])
        .send()
        .await?;

    match res.text().await {
        Ok(buf) => {
            if buf == "[]" {
                Err(anyhow!("empty response?"))
            } else {
                Ok(buf)
            }
        }
        Err(err) => Err(err.into()),
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Response {
    ver: String,
    info: Info,
    renderpass: Vec<RenderPass>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Info {
    name: String,
    //id: String,
    //date: String,
    //viewed: u32,
    //username: String,
    //description: String,
    //likes: u32,
    //published: u32,
    //flags: u32,
    //use_preview: u32,
    //tags: Vec<String>,
    //hasliked: u32,
    //parentid: String,
    //parentname: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RenderPass {
    name: String,
    code: String,
    inputs: Vec<RenderInput>,
    outputs: Vec<RenderOutput>,
    //description: String,
    //r#type: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RenderInput {
    channel: u32,
    filepath: String,
    sampler: Sampler,
    //id: String,
    //previewfilepath: String,
    //r#type: String,
    //published: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RenderOutput {
    id: String,
    channel: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Sampler {
    filter: String,
    wrap: String,
    vflip: String,
    srgb: String,
    internal: String,
}
