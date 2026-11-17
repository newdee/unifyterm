# Written by uni-term each time it starts: changes here are lost.
# fish runs this after its own start files, in uni-term's tabs only.
# `[history] enabled = false` in uni-term's settings turns it off.
#
# It tells uni-term, in standard terminal marks, where the prompt starts
# and ends (OSC 133;A and B), when a command starts (133;C), what it was
# and how it ended (633;E, 133;D) and which directory this is (633;P;Cwd).
# Adapted from keepane's shell hooks (MIT, same author).

function __uni_term_escape
    string replace -a '\\' '\\\\' -- $argv[1] | string replace -a ';' '\\x3b' | string join '\\x0a'
end

function __uni_term_preexec --on-event fish_preexec
    printf '\e]633;E;%s\e\\\e]133;C\e\\' (__uni_term_escape "$argv[1]")
end

function __uni_term_postexec --on-event fish_postexec
    printf '\e]133;D;%s\e\\' $status
end

function __uni_term_cwd --on-event fish_prompt
    printf '\e]633;P;Cwd=%s\e\\' (__uni_term_escape "$PWD")
end

if functions -q fish_prompt
    functions -c fish_prompt __uni_term_user_prompt
    function fish_prompt
        printf '\e]133;A\e\\'
        __uni_term_user_prompt
        printf '\e]133;B\e\\'
    end
end

printf '\e]633;P;Shell=fish\e\\'
