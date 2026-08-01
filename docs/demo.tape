# vhs tape for the README demo GIF.
# Regenerate with: vhs docs/demo.tape  (run from the repo root; needs a release build)

Output docs/demo.gif

Set FontSize 13
Set Width 1200
Set Height 640
Set Theme "Catppuccin Mocha"
Set Padding 12
Set TypingSpeed 40ms

Hide
Type "alias tcld=./target/release/tcld && clear"
Enter
Show

Type "tcld explore testing/fixtures/public/public_complex_fixture.tbc"
Sleep 500ms
Enter
Sleep 2s

# Walk the instruction stream; the inspector trace follows the cursor
Type "jjjj"
Sleep 800ms
Type "jjjj"
Sleep 1.2s

# Jump to the decompiled source view
Type "d"
Sleep 2s
Type "jjjjjj"
Sleep 1.5s

# Literals view
Type "l"
Sleep 1.5s
Type "jjj"
Sleep 1s

# Back to instructions, search for an opcode
Type "i"
Sleep 800ms
Type "/"
Sleep 400ms
Type "foreach"
Sleep 600ms
Enter
Sleep 2s

# Toggle the stack-effect trace
Type "t"
Sleep 2.5s
