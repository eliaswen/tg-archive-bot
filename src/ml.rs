use crate::Error;
use image::{DynamicImage, ImageFormat, ImageReader, Limits, imageops::FilterType};
use ndarray::{Array, Array4};
use ort::{
    ep::{CUDA, ExecutionProvider},
    session::Session,
    value::Tensor,
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CStr, c_char, c_void},
    env, fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tokenizers::Tokenizer;
use tracing::{error, info};

const DEFAULT_REPOSITORY: &str = "onnx-community/siglip2-base-patch16-224-ONNX";
const DEFAULT_REVISION: &str = "ba1f3b0843f24bc5417d38e19c37b287d719b2f4";
const DEFAULT_VISION_ARTIFACT: &str = "onnx/vision_model_fp16.onnx";
const DEFAULT_VISION_SHA256: &str =
    "a1959f7bd3993a607e48839f6d01e25b876fe76afda301b028b78eef68aabd95";
const DEFAULT_TEXT_ARTIFACT: &str = "onnx/text_model_quantized.onnx";
const DEFAULT_TEXT_SHA256: &str =
    "3a0603d3a00c05a80a6ded4743c16aaac7b1e62cdcc7e362e7ce418659b96400";
const DEFAULT_TOKENIZER_SHA256: &str =
    "cb9140fae3ac5122c972d37adf83e1248471a38147ad76f8215c8872c6fd8322";
const DEFAULT_PREPROCESSOR_SHA256: &str =
    "9b36b57ebaf20f09bf4c22100ccc21877ea6bfe5aead0c00c59f8af8ccefacfc";
const DEFAULT_TOKENIZER_CONFIG_SHA256: &str =
    "7c3a247599e741bceba1a3fe0285aea88d1044dc1fad2caa1e48cdd9fd25f630";
const IMAGE_SIZE: u32 = 224;
const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 12_000;
const EMBEDDING_DIMENSIONS: usize = 768;

pub struct TextEncoder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

pub struct ImageEncoder {
    session: Mutex<Session>,
    preprocessor: ImagePreprocessor,
}

#[derive(serde::Deserialize)]
struct PreprocessorConfig {
    size: PreprocessorSize,
    image_mean: [f32; 3],
    image_std: [f32; 3],
    resample: u32,
}

#[derive(serde::Deserialize)]
struct PreprocessorSize {
    height: u32,
    width: u32,
}

struct ImagePreprocessor {
    image_mean: [f32; 3],
    image_std: [f32; 3],
    filter: FilterType,
}

fn setting(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn cache_dir() -> PathBuf {
    PathBuf::from(setting(
        "TG_BOT_ML_CACHE",
        "/var/cache/tg-archive-bot/models",
    ))
}

fn model_revision() -> String {
    setting("TG_BOT_ML_REVISION", DEFAULT_REVISION)
}

pub fn model_identifier() -> String {
    format!(
        "{}@{}",
        setting("TG_BOT_ML_REPOSITORY", DEFAULT_REPOSITORY),
        model_revision()
    )
}

fn verify_override(name: &str, default: &str) -> Result<String, Error> {
    let value = setting(name, default);
    if value != default && env::var(format!("{name}_SHA256")).is_err() {
        return Err(std::io::Error::other(format!(
            "{name} override requires a matching {name}_SHA256 value"
        ))
        .into());
    }
    Ok(value)
}

fn artifact(name: &str, default_path: &str, default_hash: &str) -> Result<(String, String), Error> {
    let path = verify_override(name, default_path)?;
    let repository_override = env::var("TG_BOT_ML_REPOSITORY").is_ok();
    let revision_override = env::var("TG_BOT_ML_REVISION").is_ok();
    if (repository_override || revision_override) && env::var(format!("{name}_SHA256")).is_err() {
        return Err(std::io::Error::other(format!(
            "repository or revision overrides require a checksum for {name}"
        ))
        .into());
    }
    let hash = setting(&format!("{name}_SHA256"), default_hash);
    Ok((path, hash))
}

async fn download_artifact(path: &str, expected_hash: &str) -> Result<PathBuf, Error> {
    let repository = setting("TG_BOT_ML_REPOSITORY", DEFAULT_REPOSITORY);
    let revision = model_revision();
    let root = cache_dir().join(format!("{repository}/{revision}"));
    let target = root.join(path);
    if target.exists() && hash_file(&target)? == expected_hash {
        return Ok(target);
    }
    if target.exists() {
        fs::remove_file(&target)?;
    }
    fs::create_dir_all(
        target
            .parent()
            .ok_or_else(|| std::io::Error::other("invalid model artifact path"))?,
    )?;
    let url = format!("https://huggingface.co/{repository}/resolve/{revision}/{path}");
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let bytes = response.bytes().await?;
    if hex_hash(&bytes) != expected_hash {
        return Err(
            std::io::Error::other(format!("model artifact checksum mismatch: {path}")).into(),
        );
    }
    let temporary = target.with_extension("download");
    fs::write(&temporary, &bytes)?;
    fs::rename(temporary, &target)?;
    Ok(target)
}

fn hash_file(path: &Path) -> Result<String, Error> {
    Ok(hex_hash(&fs::read(path)?))
}

fn hex_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn tokenizer_file() -> Result<PathBuf, Error> {
    let (path, hash) = artifact(
        "TG_BOT_ML_TOKENIZER",
        "tokenizer.json",
        DEFAULT_TOKENIZER_SHA256,
    )?;
    download_artifact(&path, &hash).await
}

pub async fn load_text_encoder() -> Result<TextEncoder, Error> {
    check_onnx_runtime()?;
    let (model, hash) = artifact(
        "TG_BOT_ML_TEXT_ARTIFACT",
        DEFAULT_TEXT_ARTIFACT,
        DEFAULT_TEXT_SHA256,
    )?;
    let model = download_artifact(&model, &hash).await?;
    let (preprocessor, preprocessor_hash) = artifact(
        "TG_BOT_ML_PREPROCESSOR",
        "preprocessor_config.json",
        DEFAULT_PREPROCESSOR_SHA256,
    )?;
    let _ = download_artifact(&preprocessor, &preprocessor_hash).await?;
    let (tokenizer_config, tokenizer_config_hash) = artifact(
        "TG_BOT_ML_TOKENIZER_CONFIG",
        "tokenizer_config.json",
        DEFAULT_TOKENIZER_CONFIG_SHA256,
    )?;
    let _ = download_artifact(&tokenizer_config, &tokenizer_config_hash).await?;
    let tokenizer_path = tokenizer_file().await?;
    let tokenizer = Tokenizer::from_file(tokenizer_path)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let session = Session::builder()?.commit_from_file(model)?;
    validate_session(&session, &["input_ids"], "pooler_output")?;
    Ok(TextEncoder {
        session: Mutex::new(session),
        tokenizer,
    })
}

pub async fn load_image_encoder() -> Result<ImageEncoder, Error> {
    check_onnx_runtime()?;
    let (model, hash) = artifact(
        "TG_BOT_ML_VISION_ARTIFACT",
        DEFAULT_VISION_ARTIFACT,
        DEFAULT_VISION_SHA256,
    )?;
    let model = download_artifact(&model, &hash).await?;
    let (preprocessor_path, preprocessor_hash) = artifact(
        "TG_BOT_ML_PREPROCESSOR",
        "preprocessor_config.json",
        DEFAULT_PREPROCESSOR_SHA256,
    )?;
    let preprocessor_path = download_artifact(&preprocessor_path, &preprocessor_hash).await?;
    let preprocessor: PreprocessorConfig = serde_json::from_slice(&fs::read(preprocessor_path)?)?;
    if preprocessor.size.height != IMAGE_SIZE || preprocessor.size.width != IMAGE_SIZE {
        return Err(
            std::io::Error::other("SigLIP image preprocessing must use 224 by 224 inputs").into(),
        );
    }
    let filter = match preprocessor.resample {
        2 => FilterType::Triangle,
        3 => FilterType::CatmullRom,
        _ => return Err(std::io::Error::other("unsupported SigLIP image resampling mode").into()),
    };
    if !CUDA::default().is_available()? {
        return Err(
            std::io::Error::other("ONNX Runtime CUDA execution provider is unavailable").into(),
        );
    }
    let session = Session::builder()?
        .with_execution_providers([CUDA::default().build()])
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .commit_from_file(model)?;
    validate_session(&session, &["pixel_values"], "pooler_output")?;
    Ok(ImageEncoder {
        session: Mutex::new(session),
        preprocessor: ImagePreprocessor {
            image_mean: preprocessor.image_mean,
            image_std: preprocessor.image_std,
            filter,
        },
    })
}

fn check_onnx_runtime() -> Result<(), Error> {
    let library = setting("ORT_DYLIB_PATH", "libonnxruntime.so");
    let library_path = Path::new(&library);
    let mut candidates = Vec::new();
    if library_path.is_absolute() {
        candidates.push(library_path.to_path_buf());
    } else if library_path.components().count() > 1 {
        candidates.push(
            env::current_exe()?
                .parent()
                .unwrap_or(Path::new("."))
                .join(library_path),
        );
        candidates.push(library_path.to_path_buf());
    } else {
        if let Some(directory) = env::current_exe()?.parent() {
            candidates.push(directory.join(library_path));
        }
        if let Some(paths) = env::var_os("LD_LIBRARY_PATH") {
            candidates.extend(
                env::split_paths(&paths).map(|directory| directory.join(library_path)),
            );
        }
        for directory in [
            "/usr/local/lib",
            "/usr/lib",
            "/lib",
            "/usr/lib/x86_64-linux-gnu",
            "/lib/x86_64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/lib/aarch64-linux-gnu",
        ] {
            candidates.push(Path::new(directory).join(library_path));
        }
    }
    if !candidates.iter().any(|candidate| candidate.is_file()) {
        return Err(std::io::Error::other(format!(
            "ONNX Runtime library `{library}` was not found; install ONNX Runtime or set ORT_DYLIB_PATH"
        ))
        .into());
    }
    let runtime = unsafe { libloading::Library::new(&library) }
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    #[repr(C)]
    struct OrtApiBase {
        _get_api: unsafe extern "C" fn(u32) -> *const c_void,
        get_version_string: unsafe extern "C" fn() -> *const c_char,
    }
    type OrtGetApiBase = unsafe extern "C" fn() -> *const OrtApiBase;
    let get_api_base = unsafe { runtime.get::<OrtGetApiBase>(b"OrtGetApiBase\0") }
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let api_base = unsafe { get_api_base() };
    if api_base.is_null() {
        return Err(std::io::Error::other("ONNX Runtime returned an invalid API base").into());
    }
    let version = unsafe { CStr::from_ptr(((*api_base).get_version_string)()) }
        .to_string_lossy()
        .into_owned();
    if !version.starts_with("1.28.") {
        return Err(std::io::Error::other(format!(
            "ONNX Runtime 1.28 is required, found {version}"
        ))
        .into());
    }
    Ok(())
}

fn validate_session(
    session: &Session,
    required_inputs: &[&str],
    required_output: &str,
) -> Result<(), Error> {
    for input in required_inputs {
        if !session.inputs().iter().any(|value| value.name() == *input) {
            return Err(std::io::Error::other(format!(
                "SigLIP model is missing the `{input}` input"
            ))
            .into());
        }
    }
    if !session
        .outputs()
        .iter()
        .any(|value| value.name() == required_output)
    {
        return Err(std::io::Error::other(format!(
            "SigLIP model is missing the `{required_output}` output"
        ))
        .into());
    }
    Ok(())
}

impl TextEncoder {
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        if text.trim().is_empty() {
            return Err(std::io::Error::other("Image search query cannot be empty").into());
        }
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let pad_id = self
            .tokenizer
            .get_padding()
            .map(|padding| padding.pad_id)
            .unwrap_or(0);
        let mut ids = encoding
            .get_ids()
            .iter()
            .take(64)
            .map(|id| i64::from(*id))
            .collect::<Vec<_>>();
        ids.resize(64, i64::from(pad_id));
        let input_ids = Array::from_shape_vec((1, 64), ids)?;
        let input_ids = Tensor::from_array(input_ids)?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| std::io::Error::other("text encoder lock was poisoned"))?;
        let outputs = session.run(ort::inputs!["input_ids" => input_ids])?;
        let output = outputs
            .get("pooler_output")
            .ok_or_else(|| std::io::Error::other("SigLIP text embedding output is unavailable"))?;
        let (_, values) = output.try_extract_tensor::<f32>()?;
        normalize_embedding(values.to_vec())
    }
}

impl ImageEncoder {
    pub fn embed(&self, bytes: &[u8]) -> Result<Vec<f32>, Error> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(std::io::Error::other("image exceeds the maximum supported size").into());
        }
        let format = image::guess_format(bytes)?;
        if !matches!(
            format,
            ImageFormat::Png
                | ImageFormat::Jpeg
                | ImageFormat::Gif
                | ImageFormat::WebP
                | ImageFormat::Bmp
        ) {
            return Err(std::io::Error::other("unsupported raster image format").into());
        }
        let mut reader = ImageReader::with_format(std::io::Cursor::new(bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
        limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode()?;
        let input = preprocess_image(image, &self.preprocessor)?;
        let input = Tensor::from_array(input)?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| std::io::Error::other("image encoder lock was poisoned"))?;
        let outputs = session.run(ort::inputs!["pixel_values" => input])?;
        let output = outputs
            .get("pooler_output")
            .ok_or_else(|| std::io::Error::other("SigLIP image embedding output is unavailable"))?;
        let (_, values) = output.try_extract_tensor::<f32>()?;
        normalize_embedding(values.to_vec())
    }
}

fn preprocess_image(
    image: DynamicImage,
    preprocessor: &ImagePreprocessor,
) -> Result<Array4<f32>, Error> {
    let image = image
        .resize_exact(IMAGE_SIZE, IMAGE_SIZE, preprocessor.filter)
        .to_rgb8();
    let mut pixels = vec![0.0_f32; 3 * IMAGE_SIZE as usize * IMAGE_SIZE as usize];
    let plane = IMAGE_SIZE as usize * IMAGE_SIZE as usize;
    for (index, pixel) in image.pixels().enumerate() {
        pixels[index] =
            (pixel[0] as f32 / 255.0 - preprocessor.image_mean[0]) / preprocessor.image_std[0];
        pixels[plane + index] =
            (pixel[1] as f32 / 255.0 - preprocessor.image_mean[1]) / preprocessor.image_std[1];
        pixels[plane * 2 + index] =
            (pixel[2] as f32 / 255.0 - preprocessor.image_mean[2]) / preprocessor.image_std[2];
    }
    Ok(Array::from_shape_vec(
        (1, 3, IMAGE_SIZE as usize, IMAGE_SIZE as usize),
        pixels,
    )?)
}

fn normalize_embedding(mut values: Vec<f32>) -> Result<Vec<f32>, Error> {
    if values.len() != EMBEDDING_DIMENSIONS || values.iter().any(|value| !value.is_finite()) {
        return Err(std::io::Error::other("SigLIP returned an invalid embedding").into());
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(std::io::Error::other("SigLIP returned a zero embedding").into());
    }
    for value in &mut values {
        *value /= norm;
    }
    Ok(values)
}

pub async fn run_worker(pool: sqlx::PgPool) -> Result<(), Error> {
    let encoder = std::sync::Arc::new(load_image_encoder().await?);
    let model_revision = model_identifier();
    let batch_size = env::var("TG_BOT_ML_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(8)
        .clamp(1, 64);
    let mut reconcile_at = tokio::time::Instant::now();
    info!("SigLIP 2 CUDA image worker started");
    loop {
        if tokio::time::Instant::now() >= reconcile_at {
            match reconcile_jobs(&pool).await {
                Ok(()) => {
                    reconcile_at = tokio::time::Instant::now() + std::time::Duration::from_secs(300)
                }
                Err(error) => {
                    error!("Could not reconcile image embedding jobs: {error}");
                    reconcile_at = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                }
            }
        }
        let jobs = match claim_jobs(&pool, batch_size, &model_revision).await {
            Ok(jobs) => jobs,
            Err(error) => {
                error!("Could not claim image embedding jobs: {error}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };
        if jobs.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }
        for (message_id, version, attachment_id, data) in jobs {
            let processing_started = std::time::Instant::now();
            let encoder = encoder.clone();
            let embedding = tokio::task::spawn_blocking(move || encoder.embed(&data)).await;
            let result = match embedding {
                Ok(result) => result,
                Err(error) => Err(std::io::Error::other(error.to_string()).into()),
            };
            let result = match result {
                Ok(embedding) => {
                    persist_embedding(
                        &pool,
                        message_id,
                        version,
                        attachment_id,
                        embedding,
                        &model_revision,
                        processing_started
                            .elapsed()
                            .as_millis()
                            .min(i64::MAX as u128) as i64,
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                error!("Could not process image attachment {attachment_id}: {error}");
                if let Err(mark_error) = mark_failed(
                    &pool,
                    message_id,
                    version,
                    attachment_id,
                    &model_revision,
                    &error.to_string(),
                )
                .await
                {
                    error!("Could not reschedule image attachment {attachment_id}: {mark_error}");
                }
            }
        }
    }
}

async fn persist_embedding(
    pool: &sqlx::PgPool,
    message_id: i64,
    version: i64,
    attachment_id: i64,
    embedding: Vec<f32>,
    model_revision: &str,
    processing_duration_ms: i64,
) -> Result<(), Error> {
    let vector = pgvector::Vector::from(embedding);
    let mut transaction = pool.begin().await?;
    let desired_model_revision = sqlx::query_scalar::<_, String>("SELECT desired_model_revision FROM image_embedding_jobs WHERE message_id = $1 AND message_version = $2 AND attachment_id = $3 FOR UPDATE")
        .bind(message_id).bind(version).bind(attachment_id).fetch_optional(&mut *transaction).await?;
    if desired_model_revision.as_deref() != Some(model_revision) {
        transaction.rollback().await?;
        return Ok(());
    }
    sqlx::query("INSERT INTO image_embeddings (message_id, message_version, attachment_id, embedding, model_revision, processing_duration_ms) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (message_id, message_version, attachment_id) DO UPDATE SET embedding = EXCLUDED.embedding, model_revision = EXCLUDED.model_revision, processing_duration_ms = EXCLUDED.processing_duration_ms, processed_at = NOW()")
        .bind(message_id).bind(version).bind(attachment_id).bind(vector).bind(model_revision).bind(processing_duration_ms).execute(&mut *transaction).await?;
    sqlx::query("DELETE FROM image_embedding_jobs WHERE message_id = $1 AND message_version = $2 AND attachment_id = $3")
        .bind(message_id).bind(version).bind(attachment_id).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(())
}

async fn reconcile_jobs(pool: &sqlx::PgPool) -> Result<(), Error> {
    sqlx::query("INSERT INTO image_embedding_jobs (message_id, message_version, attachment_id, desired_model_revision) SELECT a.message_id, a.message_version, a.attachment_id, $1 FROM attachments a LEFT JOIN image_embeddings e ON e.message_id = a.message_id AND e.message_version = a.message_version AND e.attachment_id = a.attachment_id AND e.model_revision = $1 WHERE e.attachment_id IS NULL AND (a.content_type IN ('image/jpeg', 'image/jpg', 'image/png', 'image/gif', 'image/webp', 'image/bmp', 'image/x-ms-bmp') OR ((a.width IS NOT NULL AND a.height IS NOT NULL) AND a.filename ~* '\\.(jpe?g|png|gif|webp|bmp)$')) ON CONFLICT (message_id, message_version, attachment_id) DO UPDATE SET desired_model_revision = EXCLUDED.desired_model_revision, attempts = 0, available_at = NOW(), locked_until = NULL, last_error = NULL, failed_at = NULL WHERE image_embedding_jobs.desired_model_revision <> EXCLUDED.desired_model_revision")
        .bind(model_identifier()).execute(pool).await?;
    Ok(())
}

async fn claim_jobs(
    pool: &sqlx::PgPool,
    limit: i64,
    model_revision: &str,
) -> Result<Vec<(i64, i64, i64, Vec<u8>)>, Error> {
    let mut transaction = pool.begin().await?;
    let jobs = sqlx::query_as::<_, (i64, i64, i64, Vec<u8>)>("WITH claimed AS (SELECT j.message_id, j.message_version, j.attachment_id FROM image_embedding_jobs j WHERE j.desired_model_revision = $1 AND j.failed_at IS NULL AND j.available_at <= NOW() AND (j.locked_until IS NULL OR j.locked_until <= NOW()) ORDER BY j.available_at, j.created_at FOR UPDATE SKIP LOCKED LIMIT $2) UPDATE image_embedding_jobs j SET attempts = attempts + 1, locked_until = NOW() + INTERVAL '5 minutes' FROM claimed c JOIN attachments a ON a.message_id = c.message_id AND a.message_version = c.message_version AND a.attachment_id = c.attachment_id WHERE j.message_id = c.message_id AND j.message_version = c.message_version AND j.attachment_id = c.attachment_id RETURNING j.message_id, j.message_version, j.attachment_id, a.data")
        .bind(model_revision).bind(limit).fetch_all(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(jobs)
}

async fn mark_failed(
    pool: &sqlx::PgPool,
    message_id: i64,
    version: i64,
    attachment_id: i64,
    model_revision: &str,
    message: &str,
) -> Result<(), Error> {
    sqlx::query("UPDATE image_embedding_jobs SET last_error = $5, locked_until = NULL, available_at = NOW() + LEAST(POWER(2, attempts), 300) * INTERVAL '1 second', failed_at = CASE WHEN attempts >= 10 THEN NOW() ELSE NULL END WHERE message_id = $1 AND message_version = $2 AND attachment_id = $3 AND desired_model_revision = $4")
        .bind(message_id).bind(version).bind(attachment_id).bind(model_revision).bind(message).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_is_normalized_and_must_have_the_expected_shape() {
        let mut values = vec![0.0_f32; EMBEDDING_DIMENSIONS];
        values[0] = 3.0;
        values[1] = 4.0;
        let normalized = normalize_embedding(values).unwrap();
        assert_eq!(normalized[0], 0.6);
        assert_eq!(normalized[1], 0.8);
        assert!((normalized.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(normalize_embedding(vec![1.0]).is_err());
    }

    #[test]
    fn embedding_rejects_non_finite_and_zero_values() {
        assert!(normalize_embedding(vec![f32::NAN; EMBEDDING_DIMENSIONS]).is_err());
        assert!(normalize_embedding(vec![0.0; EMBEDDING_DIMENSIONS]).is_err());
    }

    #[test]
    fn image_preprocessing_produces_siglip_input_shape_and_range() {
        let image =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([255, 127, 0])));
        let preprocessor = ImagePreprocessor {
            image_mean: [0.5; 3],
            image_std: [0.5; 3],
            filter: FilterType::Triangle,
        };
        let tensor = preprocess_image(image, &preprocessor).unwrap();
        assert_eq!(tensor.shape(), &[1, 3, 224, 224]);
        assert_eq!(tensor[[0, 0, 0, 0]], 1.0);
        assert!((tensor[[0, 1, 0, 0]] - (127.0 / 255.0 * 2.0 - 1.0)).abs() < 1e-6);
        assert_eq!(tensor[[0, 2, 0, 0]], -1.0);
    }

    #[tokio::test]
    #[ignore = "requires a CUDA-enabled ONNX Runtime and NVIDIA GPU"]
    async fn cuda_smoke_test_embeds_a_fixture_image() {
        let encoder = load_image_encoder().await.unwrap();
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            224,
            224,
            image::Rgb([80, 120, 180]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        let embedding = encoder.embed(bytes.get_ref()).unwrap();
        assert_eq!(embedding.len(), EMBEDDING_DIMENSIONS);
        assert!((embedding.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    #[ignore = "requires an ONNX Runtime library and downloaded model artifacts"]
    async fn cpu_text_smoke_test_embeds_a_query() {
        let encoder = load_text_encoder().await.unwrap();
        let embedding = encoder.embed("a sunset over the ocean").unwrap();
        assert_eq!(embedding.len(), EMBEDDING_DIMENSIONS);
        assert!((embedding.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 1e-5);
    }
}
