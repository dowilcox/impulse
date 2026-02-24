#ifndef IMPULSE_FFI_H
#define IMPULSE_FFI_H

#include <stdint.h>
#include <stdbool.h>

// Opaque handle for LSP registry
typedef struct LspRegistryHandle LspRegistryHandle;

// Memory management
void impulse_free_string(char *s);

// Monaco assets
char *impulse_ensure_monaco_extracted(void);
const char *impulse_get_editor_html(void);

// Shell integration
char *impulse_get_shell_integration_script(const char *shell);
char *impulse_get_user_login_shell(void);
char *impulse_get_user_login_shell_name(void);

// Search
char *impulse_search_files(const char *root, const char *query);
char *impulse_search_content(const char *root, const char *query, bool case_sensitive);

// LSP management
LspRegistryHandle *impulse_lsp_registry_new(const char *root_uri);
int32_t impulse_lsp_ensure_servers(LspRegistryHandle *handle, const char *language_id, const char *file_uri);
char *impulse_lsp_request(LspRegistryHandle *handle, const char *language_id, const char *file_uri, const char *method, const char *params_json);
int32_t impulse_lsp_notify(LspRegistryHandle *handle, const char *language_id, const char *file_uri, const char *method, const char *params_json);
char *impulse_lsp_poll_event(LspRegistryHandle *handle);
void impulse_lsp_shutdown_all(LspRegistryHandle *handle);
void impulse_lsp_registry_free(LspRegistryHandle *handle);

// Managed LSP installation
char *impulse_lsp_check_status(void);
char *impulse_lsp_install(void);
bool impulse_npm_is_available(void);
char *impulse_system_lsp_status(void);

// Git
char *impulse_git_branch(const char *path);
char *impulse_git_status_for_directory(const char *path);
char *impulse_get_all_git_statuses(const char *path);
char *impulse_read_directory_with_git_status(const char *path, bool show_hidden);
char *impulse_git_diff_markers(const char *file_path);
char *impulse_git_blame(const char *file_path, uint32_t line);
int32_t impulse_git_discard_changes(const char *file_path, const char *workspace_root);

#endif
