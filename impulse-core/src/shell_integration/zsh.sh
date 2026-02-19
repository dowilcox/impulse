# Impulse shell integration for zsh
__impulse_precmd() {
    local exit_code=$?
    printf '\e]133;D;%d\a' "$exit_code"
    printf '\e]7;file://%s%s\a' "$HOST" "$PWD"
    printf '\e]133;A\a'
}
__impulse_preexec() {
    printf '\e]133;B\a'
    printf '\e]133;C\a'
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd __impulse_precmd
add-zsh-hook preexec __impulse_preexec
