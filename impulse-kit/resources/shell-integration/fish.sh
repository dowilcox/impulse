# Impulse shell integration for fish
# The terminal's secret, sent with every mark this reports, so what programs
# print can't pass for a mark or a command. Not exported: programs run here
# don't see it.
set -q __impulse_nonce; or set -g __impulse_nonce "$IMPULSE_SHELL_NONCE"
set -e IMPULSE_SHELL_NONCE
set -g __impulse_command_started ""
set -g __impulse_names_sig ""
set -g __impulse_path_sent ""
function __impulse_urlencode
    string escape --style=url -- $argv[1]
end
# Commands that aren't files on PATH (functions, builtins) and PATH itself,
# for the input bar's unknown-command underline. Sent again only on change.
function __impulse_report_names
    set -l names (functions --all --names) (builtin --names)
    set -l sig (count $names)
    if test "$sig" != "$__impulse_names_sig"
        set -g __impulse_names_sig $sig
        set -l joined (string join ' ' -- $names | string replace -ra '[^[:graph:] ]' '')
        printf '\e]6973;Names=%s;Nonce=%s\a' (string sub --length 60000 -- "$joined") "$__impulse_nonce"
    end
    set -l path (string join ':' -- $PATH)
    if test "$path" != "$__impulse_path_sent"
        set -g __impulse_path_sent $path
        printf '\e]6973;Path=%s;Nonce=%s\a' "$path" "$__impulse_nonce"
    end
end
function __impulse_prompt --on-event fish_prompt
    set -l exit_code $status
    if test -n "$__impulse_command_started"
        printf '\e]133;D;%d;Nonce=%s\a' $exit_code "$__impulse_nonce"
        set -g __impulse_command_started ""
    end
    printf '\e]7;file://%s%s;Nonce=%s\a' (hostname) (__impulse_urlencode $PWD) "$__impulse_nonce"
    __impulse_report_names
    printf '\e]133;A;Nonce=%s\a' "$__impulse_nonce"
end
function __impulse_preexec --on-event fish_preexec
    set -l command $argv[1]
    set -g __impulse_command_started 1
    printf '\e]6973;Command=%s;Nonce=%s\a' (__impulse_urlencode $command) $__impulse_nonce
    printf '\e]133;C;Nonce=%s\a' "$__impulse_nonce"
end
