# The markdown buffer

Some **strong** and *emphasis* with `code` and a [link](other.md). This paragraph is long enough that it should wrap across the width of the pane when the buffer is rendered, rather than running off the edge to the right.

## A list

- item one
- item two with **bold**
- [ ] a task
- [x] a done task

1. first
2. second

> a quoted line with *emphasis*

```rust
fn main() {
    println!("hi");
}
```

| name | value |
|------|-------|
| alpha | 1 |
| b | 22 |

---

![a picture](rendered.png)

Setext heading
--------------

The end.

| a very long column heading that is wider than the pane | b | another rather long column heading to make it wide |
|---|---|---|
| x | y | z |

| ![one](rendered.png) | ![two](rendered.png) |

| Light | Dark, and a heading wider |
|---|---|
| ![l](rendered.png) | ![d](rendered.png) |

Back to [the top](#the-markdown-buffer).
