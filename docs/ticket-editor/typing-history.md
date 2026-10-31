# Typing history

How undo should behave while an operator is typing in a text box. Creating a text box, a rectangle, a QR, or a barcode is already one event on the page history (`page-history.ts`): ⌘Z removes that box and ⌘⇧Z puts it back. Typing is not that yet. This note is the design for it.

The shape comes from what the editor has to do, checked against the way CodeMirror, ProseMirror, Figma, and Illustrator split a text edit from the rest of the document.

## Text stack and page stack

Undo has two stacks, and only one of them hears ⌘Z at a time. The page stack is the one the sheet uses. The text stack exists only while a draft is open.

The **text stack** exists only while the text draft is open. It remembers the string, not the page. ⌘Z and ⌘⇧Z walk that stack. They do not remove the box, and they do not undo a QR that was created before the draft opened.

The **page stack** is the history the sheet uses. A created text box, rectangle, QR, or barcode is one event. A drag or a resize is one event on that same stack. ⌘Z from the sheet walks it. There is no second canvas history.

Leaving the text closes the text stack. If the string changed, every block from that visit becomes **one** event on the page stack. The next ⌘Z restores the string from the moment the draft opened. ⌘⇧Z puts the finished string back. The per-character blocks are gone.

## While the draft is open

A single character is one undo step when it stands alone. Characters typed together become one block, and ⌘Z removes that block in one step.

A character joins the current block when all of these are true:

- It is an insert, and the current block is an insert.
- It arrives within 500 ms of the previous change in that block. CodeMirror and ProseMirror use this same delay (`newGroupDelay`). A longer pause starts a new block, so a letter typed after a pause is its own step.
- The caret has not moved, except for the move caused by the insert itself. An arrow key, a click in the text, or a selection closes the block. The next characters start another one.
- The insert does not cross a word boundary. A space, a tab, or a line break ends the block. The separator is the last character of that block. The next word is the next block. A fast sentence then undoes a word at a time, not the whole sentence and not one letter at a time.

Delete and backspace never join an insert block. A run of deletes inside the 500 ms window, with the caret only moving because of the delete, is one delete block.

A paste, a cut, or a variable dropped into the text is its own block. It is not split into characters.

An IME composition is one block when the composition commits. Intermediate composition updates do not each become a step.

Redo inside the draft walks those same blocks forward. A new change after an undo drops the redo side of the text stack.

⌘Z inside the draft stops at the string the draft opened with. It does not continue into the page stack while the draft is still open.

## When the visit ends

The text stack folds into one page event when the operator finishes the visit:

- Click outside the text, which blurs the draft.
- Enter. The editor inserts the line break and then closes the draft, so Enter is a finish, not another open block.
- A move or a resize of that box. The fold happens first, then the move is the next event on the page stack. The first ⌘Z after a move puts the box back. The next ⌘Z restores the text from before the visit.

Escape cancels the draft. The string returns to what it was when the draft opened, the text stack is thrown away, and the page stack gains nothing.

If the visit ends on the same string it started with, the page stack gains nothing.

The page event stores the whole page from before the visit, the same way a new QR stores the page from before the drag. One ⌘Z restores that page. One ⌘⇧Z returns the finished text. Save still writes the file only when the operator saves.

## What this avoids

Do not push every key onto the page stack. A finished sentence would then take one undo per letter even after the operator had clicked away.

Do not keep the character blocks after the visit. Figma and Illustrator collapse the text edit when you leave the text. The next undo is the whole edit. Keeping the blocks after a click away is the document-editor model (a long Google Doc), and it fights "everything that was typed became one event."

Do not coalesce insert with delete. Undoing a correction would delete the word the operator had just retyped.

Do not treat a cursor move as an undo step. It only closes the current block.
