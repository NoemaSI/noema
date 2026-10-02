//! The generalized mechanistic-discovery skill.
//!
//! Where the protein-binding benchmark fixes *one* experimental reality
//! (BLI binding curves, a concentration split, a fitting protocol), this
//! crate fixes the *shape* of such a reality: measured response curves
//! at held-constant inputs, a fixed visible/hidden input split, a fixed
//! simulation protocol, a fixed optimizer, and a fixed loss. The artifact
//! under test is a mechanistic Modelica candidate: the driver compiles it
//! with Rumoca through the shared [`modelica`] crate — caching by content
//! hash exactly like the plugin loader caches dylibs — fits its
//! parameters against the visible curves only, freezes the fit, and
//! scores predictions on the held-out inputs.
//!
//! Everything problem-specific lives in a *pack*: a small compiled
//! plugin exposing a [`pack::ProblemPack`] that supplies the data, the
//! train/validation/hidden split, the vocabulary ([`lexicon::Lexicon`]),
//! the baseline mechanism, and the domain text for the LLM prompts. The
//! loop, the IR, the lowerer, the optimizer drive, the scoring, and the
//! judge plumbing never change.
//!
//! The boundary is the same as everywhere else in this repository:
//!
//! > The model author controls model structure. The harness controls
//! > reality.
use anyhow::bail;
use noema_config::NoemaConfig;
use std::{fs::{copy, create_dir_all}, path::{Path, PathBuf}};


pub mod agent;
pub mod author;
pub mod bench;
pub mod candidate;
pub mod curvecheck;
pub mod data;
pub mod driver;
pub mod evolve;
pub mod export;
pub mod experiment;
pub mod fit;
pub mod grade;
pub mod jail;
pub mod ledger;
pub mod lexicon;
pub mod lower;
pub mod mechanism;
pub mod mechanism_ir;
pub mod pack;
pub mod pack_loader;
pub mod parampack;
pub mod plugin;
pub mod plot;
pub mod prepare;
pub mod reference;
pub mod split;
#[cfg(feature = "viewer")]
pub mod viewer;
pub mod workspace;

/// Re-exports so pack plugins (compiled against this crate alone) can
/// parse and fetch their data files without extra `--extern` flags.
pub use anyhow;
pub use base64;
pub use csv;
pub use serde_json;

use crate::author::{AuthorSession, ColumnMap};

/// Repository root (the directory holding `assets/`). Delegates to
/// [`modelica::workspace_root`] (compile-time manifest parent,
/// overridden by `RWSI_ROOT_DIR`).
pub fn workspace_root() -> std::path::PathBuf {
    modelica::workspace_root()
}


pub const PACK_DATA_INP_DIR: &str = "data_in";
pub const PACK_DATA_OUT_DIR: &str = "packs";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillpackIdentifier(String);

impl SkillpackIdentifier {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();

        if value.is_empty() {
            return Err("value cannot be empty".into());
        }

        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            let proposal =  value.trim().to_ascii_lowercase().replace(' ', "_");
            return Err(format!("value must contain only letters, numbers, and underscores. Consider naming it e.g. {proposal}").into());
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SkillpackIdentifier {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// returns the directory for pack data inputs
pub fn pack_data_input_dir(config: &NoemaConfig, namespace: &str) -> PathBuf {
    config.workdir.join(PACK_DATA_INP_DIR).join(namespace)
}  

/// returns the directory for pack data outputs
pub fn pack_data_output_dir(config: &NoemaConfig, namespace: &str) -> PathBuf {
    config.workdir.join(PACK_DATA_OUT_DIR).join(namespace)
}  

/// copies uploaded files into the skill workdir in preparation of llm guided pack creation
pub fn copy_uploaded_files_to_pack_input_dir(
    files_uploaded: &Vec<String>,
    skillpack_identifier: &SkillpackIdentifier,
    config: &NoemaConfig,
) -> Result<(), anyhow::Error>{
    let as_paths = files_uploaded.iter().map(|f|  PathBuf::from(f)).collect::<Vec<PathBuf>>();
    for path in as_paths {
        if !path.is_file() {
            bail!("{} is not a file", &path.display());
        }
        let from = path.clone();
        let to_folder   = pack_data_input_dir(&config, skillpack_identifier.as_str());
        create_dir_all(&to_folder)?;

        let dest_file = to_folder.join(path.file_name().expect("cant extract file name"));
        copy(from, dest_file)?;
    }

    Ok(())
}
/// creates a new skill pack using the LLM to read data and interpret it
pub fn new_skill_pack_llm_guided(
    skillpack_identifier: SkillpackIdentifier,
    problem_description: String,
    baseline_hint: Option<String>,
    config: NoemaConfig,
) -> Result<(), anyhow::Error> {
    let name = skillpack_identifier.as_str();
    let data_dir = pack_data_input_dir(&config, &name);

    let dir = pack_data_output_dir(&config, &name);
    println!("creating dir: {}", &dir.display());
    create_dir_all(&dir)?;

    let session_path = dir.join("session.json");

    if session_path.exists() {
        bail!("Session already exists")
    }

    let session = AuthorSession {
        name: name.clone().to_string(),
        problem_statement: problem_description.into(),
        baseline_hint: baseline_hint,
        prepare_brief: Some(
            "The scientist supplied only the path and the problem. Explore the data \
             and propose the complete reading yourself.".to_string(),
        ),
        data: author::DataSpec { path: data_dir.display().to_string(), delimiter: ',' },
        mapping: ColumnMap {
            subject: None,
            curve: vec![],
            time: None,
            drives: vec![],
            outputs: vec![],
            condition_column: None,
        },
        split: author::SplitDraft::default(),
    };

    author::write_session(&session_path, &session)?;
    prepare::run_proposal(
        &session_path.display().to_string(),
        None,
        Some((&config.llm).into()),

    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use noema_config::{LlmConfig, NoemaConfig};
    use super::*;

    use std::fs;
    use tempfile::tempdir;

    fn gen_noema_config() -> NoemaConfig {
        NoemaConfig {
            llm: LlmConfig {
                model_name: "gpt-4o".to_string(),
                api_key: Some("sk-...".to_string()),
                endpoint: "https://api.openai.com/v1".to_string(),
            },
            workdir: PathBuf::from("/path/to/workdir"),
        }
    }

    #[test]
    fn accepts_letters() {
        let word = SkillpackIdentifier::new("hello").unwrap();

        assert_eq!(word.as_str(), "hello");
    }

    #[test]
    fn accepts_underscores_and_numbers() {
        let word = SkillpackIdentifier::new("hello_world_123").unwrap();

        assert_eq!(word.as_str(), "hello_world_123");
    }

    #[test]
    fn accepts_a_single_underscore() {
        let word = SkillpackIdentifier::new("_").unwrap();

        assert_eq!(word.as_str(), "_");
    }

    #[test]
    fn rejects_empty_string() {
        assert!(SkillpackIdentifier::new("").is_err());
    }

    #[test]
    fn rejects_spaces() {
        assert!(SkillpackIdentifier::new("hello world").is_err());
    }

    #[test]
    fn rejects_hyphens() {
        assert!(SkillpackIdentifier::new("hello-world").is_err());
    }

    #[test]
    fn rejects_punctuation() {
        assert!(SkillpackIdentifier::new("hello!").is_err());
        assert!(SkillpackIdentifier::new("hello.world").is_err());
    }

    #[test]
    fn rejects_unicode_characters() {
        assert!(SkillpackIdentifier::new("café").is_err());
        assert!(SkillpackIdentifier::new("こんにちは").is_err());
    }

    #[test]
    fn returns_skilldata_input_dir() {
        let config = gen_noema_config();
        let inpt_dir = pack_data_input_dir(&config, "foobar");
        assert_eq!(inpt_dir, PathBuf::from("/path/to/workdir/data_in/foobar"));
    }

    #[test]
    fn returns_skilldata_output_dir() {
        let config = gen_noema_config();
        let inpt_dir = pack_data_output_dir(&config, "foobar");
        assert_eq!(inpt_dir, PathBuf::from("/path/to/workdir/packs/foobar"));
    }

    #[test]
    fn copies_single_uploaded_file() {
        let skillpack_id = SkillpackIdentifier::new("foo_bar").expect("unable to create skillpack id");
        let mut config = gen_noema_config();
        let source_dir = tempdir().expect("could not create temp directory");
        let source_path = source_dir.path().join("uploaded.txt");

        config.workdir = tempdir().expect("could not create temp directory").path().to_path_buf();

        let packdata_inpt_dir = pack_data_input_dir(&config, "foo_bar");
        let destination_path = packdata_inpt_dir.join("uploaded.txt");

        let contents = b"test uploaded file contents";
        fs::write(&source_path, contents).expect("could not create source file");

        let files_uploaded = vec![source_path.display().to_string()];
        copy_uploaded_files_to_pack_input_dir(
            &files_uploaded,
            &skillpack_id,
            &config
        ).expect("could not copy files");

        assert!(destination_path.exists());

        let copied_contents =
            fs::read(&destination_path).expect("could not read copied file");

        assert_eq!(copied_contents, contents);
        drop(source_dir);
        drop(config.workdir);
    }

    #[test]
    fn copies_multiple_uploaded_files() {
        let skillpack_id = SkillpackIdentifier::new("foo_bar").expect("unable to create skillpack id");
        let mut config = gen_noema_config();
        let source_dir = tempdir().expect("could not create temp directory");
        let contents = b"test uploaded file contents";

        config.workdir = tempdir().expect("could not create temp directory").path().to_path_buf();
        let packdata_inpt_dir = pack_data_input_dir(&config, "foo_bar");

        let dummy_files = vec!["foo_1.txt", "foo_2.txt"];
        let mut files_uploaded = vec![];

        for f in &dummy_files {
            let source_path = source_dir.path().join(f);
            fs::write(&source_path, contents).expect("could not create source file");
            files_uploaded.push(source_path.display().to_string());
        }

        copy_uploaded_files_to_pack_input_dir(
            &files_uploaded,
            &skillpack_id,
            &config
        ).expect("could not copy files");

        for f in &dummy_files {
            let dest_file = packdata_inpt_dir.join(f);
            println!("{}", &dest_file.display());
            assert!(dest_file.exists());
            let copied_contents = fs::read(&dest_file).expect("could not read copied file");
            assert_eq!(copied_contents, contents);
        }

        drop(source_dir);
        drop(config.workdir);
    }
}

