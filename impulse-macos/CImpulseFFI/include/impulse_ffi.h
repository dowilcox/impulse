#ifndef IMPULSE_FFI_H
#define IMPULSE_FFI_H

#include <stdint.h>
#include <stdbool.h>
#include <libproc.h>

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

// Markdown preview
char *impulse_render_markdown_preview(const char *source, const char *theme_json, const char *highlight_js_path);
bool impulse_is_markdown_file(const char *path);

// SVG preview
char *impulse_render_svg_preview(const char *source, const char *bg_color);
bool impulse_is_svg_file(const char *path);

// Previewable file detection (markdown or SVG)
bool impulse_is_previewable_file(const char *path);

// Git
char *impulse_git_branch(const char *path);
char *impulse_git_status_for_directory(const char *path);
char *impulse_get_all_git_statuses(const char *path);
char *impulse_read_directory_with_git_status(const char *path, bool show_hidden);
char *impulse_git_diff_markers(const char *file_path);
char *impulse_git_blame(const char *file_path, uint32_t line);
int32_t impulse_git_discard_changes(const char *file_path, const char *workspace_root);

// Settings
char *impulse_settings_default_json(void);
char *impulse_settings_load_json(const char *json);
char *impulse_settings_validate_json(const char *json);
bool impulse_matches_file_pattern(const char *path, const char *pattern);

// Update checking
char *impulse_check_for_update(void);
const char *impulse_get_version(void);

// Terminal (impulse-terminal)
typedef struct TerminalHandle TerminalHandle;

TerminalHandle *impulse_terminal_create(const char *config_json, uint16_t cols, uint16_t rows, uint16_t cell_width, uint16_t cell_height);
void impulse_terminal_destroy(TerminalHandle *handle);
void impulse_terminal_write(TerminalHandle *handle, const uint8_t *data, size_t len);
void impulse_terminal_resize(TerminalHandle *handle, uint16_t cols, uint16_t rows, uint16_t cell_width, uint16_t cell_height);
char *impulse_terminal_grid_snapshot(TerminalHandle *handle);
char *impulse_terminal_poll_events(TerminalHandle *handle);
void impulse_terminal_start_selection(TerminalHandle *handle, uint16_t col, uint16_t row, const char *kind);
void impulse_terminal_update_selection(TerminalHandle *handle, uint16_t col, uint16_t row);
void impulse_terminal_clear_selection(TerminalHandle *handle);
char *impulse_terminal_selected_text(TerminalHandle *handle);
void impulse_terminal_scroll(TerminalHandle *handle, int32_t delta);
char *impulse_terminal_mode(TerminalHandle *handle);
void impulse_terminal_set_focus(TerminalHandle *handle, bool focused);
uint32_t impulse_terminal_child_pid(TerminalHandle *handle);

#endif
