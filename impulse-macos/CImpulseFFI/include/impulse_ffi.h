#ifndef IMPULSE_FFI_H
#define IMPULSE_FFI_H

#include <stdint.h>
#include <stdbool.h>
#include <libproc.h>

// Opaque handle for LSP registry
typedef struct LspRegistryHandle LspRegistryHandle;

// Memory management
void impulse_free_string(char *s);

// Input completion
void impulse_completion_warm_cache(void);

// LSP management
LspRegistryHandle *impulse_lsp_registry_new(const char *root_uri);
int32_t impulse_lsp_ensure_servers(LspRegistryHandle *handle, const char *language_id, const char *file_uri);
char *impulse_lsp_request(LspRegistryHandle *handle, const char *language_id, const char *file_uri, const char *method, const char *params_json);
int32_t impulse_lsp_notify(LspRegistryHandle *handle, const char *language_id, const char *file_uri, const char *method, const char *params_json);
int32_t impulse_lsp_did_change(LspRegistryHandle *handle, const char *language_id, const char *file_uri, int32_t version, const char *full_text, const char *changes_json);
char *impulse_lsp_poll_event(LspRegistryHandle *handle);
void impulse_lsp_shutdown_all(LspRegistryHandle *handle);
void impulse_lsp_registry_free(LspRegistryHandle *handle);

// Managed LSP installation
char *impulse_lsp_check_status(void);
char *impulse_lsp_install(void);
bool impulse_npm_is_available(void);
char *impulse_system_lsp_status(void);

// Terminal backend API
void *impulse_terminal_create(const char *config_json, unsigned short cols, unsigned short rows, unsigned short cell_width, unsigned short cell_height);
void impulse_terminal_destroy(void *handle);
void impulse_terminal_write(void *handle, const unsigned char *data, unsigned long len);
void impulse_terminal_resize(void *handle, unsigned short cols, unsigned short rows, unsigned short cell_width, unsigned short cell_height);
unsigned long impulse_terminal_grid_snapshot(void *handle, unsigned char *out_buf, unsigned long buf_len);
unsigned long impulse_terminal_grid_snapshot_size(void *handle);
int64_t impulse_terminal_take_damage(void *handle, unsigned short *out_rows, unsigned long cap);
char *impulse_terminal_poll_events(void *handle);
char *impulse_terminal_command_blocks(void *handle);
char *impulse_terminal_block_overlay(void *handle);
unsigned int impulse_terminal_command_block_flags(void *handle);
char *impulse_terminal_command_history_search(void *handle, const char *query_json);
char *impulse_terminal_complete_input(void *handle, const char *input, const char *cwd);
// Path completion candidates for the input-bar dropdown. Returns JSON:
//   { "span": { "start": usize, "end": usize },
//     "candidates": [ { "value": string, "display": string, "kind": "path",
//                       "is_dir": bool, "git_status": string|null } ] }
// Returns NULL on error. Caller frees with impulse_free_string.
char *impulse_terminal_completion_candidates(void *handle, const char *input, const char *cwd, unsigned long limit);
_Bool impulse_terminal_rerun_command(void *handle, const char *command);
void impulse_terminal_start_selection(void *handle, unsigned short col, unsigned short row, unsigned char kind);
void impulse_terminal_update_selection(void *handle, unsigned short col, unsigned short row);
void impulse_terminal_clear_selection(void *handle);
char *impulse_terminal_selected_text(void *handle);
void impulse_terminal_scroll(void *handle, int delta);
void impulse_terminal_scroll_to_bottom(void *handle);
_Bool impulse_terminal_scroll_to_command_block(void *handle, unsigned long long block_id);
char *impulse_terminal_mode(void *handle);
void impulse_terminal_set_focus(void *handle, _Bool focused);
unsigned int impulse_terminal_child_pid(void *handle);
char *impulse_terminal_search(void *handle, const char *pattern);
char *impulse_terminal_search_next(void *handle);
char *impulse_terminal_search_prev(void *handle);
void impulse_terminal_search_clear(void *handle);
void impulse_terminal_set_colors(void *handle, const char *config_json);
char *impulse_terminal_hyperlink_at(void *handle, unsigned int col, unsigned int row);

#endif
