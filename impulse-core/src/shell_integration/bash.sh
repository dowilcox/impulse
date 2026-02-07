# Impulse shell integration for bash
__impulse_prompt_command() {
    local exit_code=$?
    printf '\e]133;D;%d\a' "$exit_code"
    printf '\e]7;file://%s%s\a' "$(hostname)" "$PWD"
    printf '\e]133;A\a'
}
__impulse_preexec() {
    printf '\e]133;B\a'
    printf '\e]133;C\a'
}
if [[ ! "$PROMPT_COMMAND" == *"__impulse_prompt_command"* ]]; then
    PROMPT_COMMAND="__impulse_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
trap '__impulse_preexec' DEBUG
