-- Quotes every argument inside AppleScript before requesting administrator authorization. Remote
-- release metadata is never interpolated into executable AppleScript or shell source.
on run argv
    if (count of argv) is less than 2 then error "Clinch updater arguments are missing."
    set commandText to ""
    repeat with argumentText in argv
        if commandText is not "" then set commandText to commandText & " "
        set commandText to commandText & quoted form of (argumentText as text)
    end repeat
    do shell script commandText with prompt "Clinch needs administrator privileges to install this verified update." with administrator privileges
end run
