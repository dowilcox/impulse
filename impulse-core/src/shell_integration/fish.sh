# Impulse shell integration for fish
function __impulse_prompt --on-event fish_prompt
    printf '\e]133;D;%d\a' $status
    printf '\e]7;file://%s%s\a' (hostname) $PWD
    printf '\e]133;A\a'
end
function __impulse_preexec --on-event fish_preexec
    printf '\e]133;B\a'
    printf '\e]133;C\a'
end
