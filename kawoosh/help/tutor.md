# Tutor

A hands-on tour of kawoosh's keys. This is your own copy, a scratch buffer with no file behind it, so change anything you like: nothing here touches a file, and `:w` is not needed. Read a lesson, then try its keys on the practice lines under it. `:tutor` gives you a fresh copy at any time.

kawoosh is modal. In normal mode, keys move and edit; in insert mode, they type. You start in normal mode, and `<Esc>` always brings you back to it.

## Lesson 1: moving

`h` `j` `k` `l` move left, down, up and right (the arrow keys work too). `w` jumps to the start of the next word, `b` back to the start of the word, and `e` to the end of the word. `0` and `$` go to the start and end of the line; `gg` and `G` go to the first and last line of the buffer. A number before a motion repeats it: `3w` moves three words.

Move down to the next line with `j`, then hop along it with `w` until you reach TARGET, and come back with `b`.
one two three four five six TARGET seven eight
Try `e` on this line, then `$` to reach its end, then `0` to go back to its start.
Now press `G` to see the end of the tutor, and `gg` to come back to the top.

## Lesson 2: inserting

`i` starts typing before the caret, `a` after it, and `A` at the end of the line. `o` opens a new line below and starts typing there. Press `<Esc>` when you are done typing.

Add the missing words so the line reads "The quick brown fox jumps over the lazy dog":
The quick fox over the dog.
Use `A` to finish this line with the words "and done":
This line is not
Put the caret on this line and press `o` to add a line of your own under it.

## Lesson 3: deleting

`x` deletes the character under the caret. `dw` deletes to the start of the next word, and `dd` deletes the whole line. A count works here too: `2dd` deletes two lines.

Fix the doubled letters with `x`:
Thhe caat ssat onn thee mat.
Delete the extra words with `dw`:
The the cat sat sat on on the mat.
Delete the next two lines with `dd`, or with `2dd`:
This line should go.
This line should go too.
This line stays.

## Lesson 4: changing

`cw` deletes from the caret to the end of the word and starts typing, the space after it kept: type the new word, then `<Esc>`. `ciw` changes just the word under the caret, wherever in it the caret is. `r` replaces the one character under the caret without leaving normal mode.

Change "dog" to "cat" and "moon" to "sun":
The dog barked at the moon all night.
Fix the typo with `r`:
The cat sxt on the mat.

## Lesson 5: undo and redo

`u` undoes the last change, and `<C-r>` (or `U`) redoes it. `.` repeats the last change where the caret is now.

Delete a word on this line with `dw`, undo it with `u`, and redo it with `<C-r>`:
Undo is always there to catch you.
Delete "very" once with `dw`, then move to the next "very" and press `.` to delete it too:
a very very very long line

## Lesson 6: selecting, yanking and putting

`v` starts selecting characters and `V` selects whole lines; move to grow the selection. With a selection, `y` yanks (copies) it, `d` deletes it and `c` changes it. `p` puts what you yanked after the caret and `P` before it. `yy` yanks the whole line without selecting it first. A yank goes to the system clipboard too.

Yank this line with `yy` and put a copy of it below with `p`.
Select the word "red" here with `v` and `e` and yank it with `y`. Then put the caret on the first of the two spaces after "are" and put the word there with `p`:
roses are red, apples are  too.
Select these two lines with `V` and `j`, then delete them with `d`:
first line to remove
second line to remove

## Lesson 7: several selections at once

`<C-n>` selects the word under the caret; each press after that adds the next place the same word appears. An edit then happens at every selection: `c` changes them all at once. `<C-j>` adds a caret on the line below, so you can type on several lines together. `<Esc>` leaves insert mode, and `<Esc>` again goes back to one caret.

Put the caret on "red", press `<C-n>` four times, then `c`, type "blue" and press `<Esc>` twice:
a red door, a red car, a red hat and a red kite.
Put the caret at the start of the first of these three lines (`0`), press `<C-j>` twice, then `i`, type "fruit: " and press `<Esc>` twice:
apple
banana
cherry

## Lesson 8: searching

`/` opens the search: type a word and press `<CR>` to jump to it. `n` goes to the next match and `N` to the previous one. `*` searches for the word under the caret. `<Esc>` in normal mode clears the highlight; `n` still remembers the word.

Search for "needle" with `/needle` and `<CR>`, then press `n` a few times:
hay hay needle hay hay
hay needle hay hay hay
hay hay hay hay needle
Put the caret on "straw" and press `*`, then `n`:
straw hay straw hay straw

## Lesson 9: the command line

`:` opens the command line at the bottom of the window. Type a command and press `<CR>` to run it, or `<Esc>` to back out. `:help` opens the help, `:help editing` opens one page of it, and `<leader>ic` (Space, then `i`, then `c`) lists every command in a picker you can search.

Try `:help` now. The help opens in the same pane; come back with `:b tutor`, or with `<leader><leader>`, which lists your buffers.

## Lesson 10: panes

`<C-w>v` makes a new pane beside this one. A new pane first asks what it is for: `<CR>` shows this same buffer in it, `s` a scratch, `t` a terminal, `d` the folder's files. `<C-w>h` and `<C-w>l` move between panes (`<C-S-h>` and `<C-S-l>` do too, from any pane), and `<C-w>q` closes the pane you are in.

Press `<C-w>v`, then `<CR>`. Edit this line in one pane and watch the other follow:
Both panes show the same buffer.
Close the new pane with `<C-w>q`.

## Lesson 11: jumping back

A big move — `gg`, `G`, a search, a file opened from a picker — is a jump, and kawoosh remembers the place it left. `<C-o>` goes back to where you were before the jump, and `<C-i>` forward again. `<leader>f` lists the files under the working directory: type part of a name to filter them, and `<CR>` opens the one under the cursor.

Press `gg` to go to the top, then `<C-o>` to come back to this line, then `<C-i>` to go to the top again and `<C-o>` once more.
Press `<leader>f`, type a few letters of a file's name and press `<CR>`; then `<C-o>` brings you back to the tutor.

## Where to go next

- `:help` (or `<leader>ih`) lists every help page; `gx` on a link follows it. [start](start.md) is a good next read, then [editing](editing.md), [panes](panes.md), [files](files.md) and [search](search.md).
- `<leader>?` shows every key you can press first, and the card at the bottom right shows what can follow a key like `<leader>`, `g` or `<C-w>`.
- `<leader>f` finds a file, `<leader><leader>` your open buffers, and `-` lists the folder of the file you are in.
- In code, `gcc` comments a line out and back in, and `gc` with a motion does the same for lines (`gcip` a paragraph); see [editing](editing.md#comments).
