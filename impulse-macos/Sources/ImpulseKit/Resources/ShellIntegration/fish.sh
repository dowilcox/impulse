# Impulse shell integration for fish
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
        printf '\e]6973;Names=%s\a' (string sub --length 60000 -- "$joined")
    end
    set -l path (string join ':' -- $PATH)
    if test "$path" != "$__impulse_path_sent"
        set -g __impulse_path_sent $path
        printf '\e]6973;Path=%s\a' $path
    end
end
function __impulse_prompt --on-event fish_prompt
    set -l exit_code $status
    if test -n "$__impulse_command_started"
        printf '\e]133;D;%d\a' $exit_code
        set -g __impulse_command_started ""
    end
    printf '\e]7;file://%s%s\a' (hostname) (__impulse_urlencode $PWD)
    __impulse_report_names
    printf '\e]133;A\a'
end
function __impulse_preexec --on-event fish_preexec
    set -l command $argv[1]
    set -g __impulse_command_started 1
    printf '\e]6973;Command=%s\a' (__impulse_urlencode $command)
    printf '\e]133;C\a'
end
