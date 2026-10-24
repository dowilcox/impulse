//! What a repository tells Impulse about itself, and what Impulse works out
//! from it (the Swift `ImpulseKit/Project` folder): the project settings
//! (`.impulse/project.toml` and the local `.git/impulse/project.toml`),
//! Project Setup's suggestions and the way it writes them back, each task's
//! own ports and values, its Compose override, the commands for changed
//! dependency files, and folders cloned into new tasks.

pub mod compose_file;
pub mod config;
pub mod dependency_changes;
pub mod detector;
pub mod folder_clone;
pub mod settings_file;
pub mod task_environment;
