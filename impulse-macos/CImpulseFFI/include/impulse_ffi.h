#ifndef IMPULSE_FFI_H
#define IMPULSE_FFI_H

#include <stdint.h>
#include <stdbool.h>
#include <libproc.h>

// Memory management
void impulse_free_string(char *s);

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
char *impulse_terminal_recent_commands(void *handle, unsigned long limit);
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
