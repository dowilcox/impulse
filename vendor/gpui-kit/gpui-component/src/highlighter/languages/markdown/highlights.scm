;From nvim-treesitter/nvim-treesitter
(atx_heading (inline) @title)
(setext_heading (paragraph) @title)

[
  (atx_h1_marker)
  (atx_h2_marker)
  (atx_h3_marker)
  (atx_h4_marker)
  (atx_h5_marker)
  (atx_h6_marker)
  (setext_h1_underline)
  (setext_h2_underline)
] @punctuation.special

[
  (link_title)
  (indented_code_block)
  (fenced_code_block)
] @text.literal

; Impulse: a code fence and its info string are strings, as in Monaco's
; Markdown tokenizer (was `@punctuation.delimiter`, the fence alone).
[
  (fenced_code_block_delimiter)
  (info_string)
] @string

; Impulse: tables as Monaco's tokenizer colors them: the header row and the
; delimiter row whole, and the pipes of the other rows, as keywords.
(pipe_table_header) @keyword
(pipe_table_delimiter_row) @keyword
(pipe_table_row "|" @keyword)

(code_fence_content) @none

[
  (link_destination)
] @text.uri

[
  (link_label)
] @text.reference

[
  (list_marker_plus)
  (list_marker_minus)
  (list_marker_star)
  (list_marker_dot)
  (list_marker_parenthesis)
  (thematic_break)
] @punctuation.list_marker

[
  (block_continuation)
  (block_quote_marker)
] @punctuation.special

[
  (backslash_escape)
] @string.escape