# Written by unifyterm each time it starts: changes here are lost.
# fish runs this after its own start files, in unifyterm's tabs only.
# `[history] enabled = false` in unifyterm's settings turns it off.
#
# It tells unifyterm, in standard terminal marks, where the prompt starts
# and ends (OSC 133;A and B), when a command starts (133;C), what it was
# and how it ended (633;E, 133;D) and which directory this is (633;P;Cwd).
# Adapted from keepane's shell hooks (MIT, same author).

function __unifyterm_escape
    string replace -a '\\' '\\\\' -- $argv[1] | string replace -a ';' '\\x3b' | string join '\\x0a'
end

function __unifyterm_preexec --on-event fish_preexec
    printf '\e]633;E;%s\e\\\e]133;C\e\\' (__unifyterm_escape "$argv[1]")
end

function __unifyterm_postexec --on-event fish_postexec
    printf '\e]133;D;%s\e\\' $status
end

function __unifyterm_cwd --on-event fish_prompt
    printf '\e]633;P;Cwd=%s\e\\' (__unifyterm_escape "$PWD")
end

if functions -q fish_prompt
    functions -c fish_prompt __unifyterm_user_prompt
    function fish_prompt
        printf '\e]133;A\e\\'
        __unifyterm_user_prompt
        printf '\e]133;B\e\\'
    end
end

printf '\e]633;P;Shell=fish\e\\'
