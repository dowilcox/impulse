# Impulse shell integration for fish
set -g __impulse_command_started ""
function __impulse_urlencode
    string escape --style=url -- $argv[1]
end
function __impulse_prompt --on-event fish_prompt
    set -l exit_code $status
    if test -n "$__impulse_command_started"
        printf '\e]133;D;%d\a' $exit_code
    end
    printf '\e]7;file://%s%s\a' (hostname) (__impulse_urlencode $PWD)
    printf '\e]133;A\a'
end
function __impulse_preexec --on-event fish_preexec
    set -g __impulse_command_started 1
    printf '\e]133;B\a'
    printf '\e]133;C\a'
end
