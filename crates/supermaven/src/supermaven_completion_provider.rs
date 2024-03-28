use crate::{Supermaven, SupermavenCompletionStateId};
use anyhow::Result;
use editor::{Direction, InlineCompletionProvider};
use futures::StreamExt;
use gpui::{AppContext, Global, Model, ModelContext, Task};
use language::{
    language_settings::all_language_settings, Anchor, Buffer, OffsetRangeExt, ToOffset,
};
use std::time::Duration;

pub const DEBOUNCE_TIMEOUT: Duration = Duration::from_millis(75);

pub struct SupermavenCompletionProvider {
    completion_id: Option<SupermavenCompletionStateId>,
    pending_refresh: Task<Result<()>>,
}

impl SupermavenCompletionProvider {
    pub fn new() -> Self {
        Self {
            completion_id: None,
            pending_refresh: Task::ready(Ok(())),
        }
    }
}

impl InlineCompletionProvider for SupermavenCompletionProvider {
    fn is_enabled(&self, buffer: &Model<Buffer>, cursor_position: Anchor, cx: &AppContext) -> bool {
        if !Supermaven::get(cx).is_enabled() {
            return false;
        }

        let buffer = buffer.read(cx);
        let file = buffer.file();
        let language = buffer.language_at(cursor_position);
        let settings = all_language_settings(file, cx);
        settings.inline_completions_enabled(language.as_ref(), file.map(|f| f.path().as_ref()))
    }

    fn refresh(
        &mut self,
        buffer_handle: Model<Buffer>,
        cursor_position: Anchor,
        debounce: bool,
        cx: &mut ModelContext<Self>,
    ) {
        let (completion_id, mut updates) = Supermaven::update(cx, |supermaven, cx| {
            supermaven.complete(&buffer_handle, cursor_position, cx)
        });

        self.pending_refresh = cx.spawn(|this, mut cx| async move {
            if debounce {
                cx.background_executor().timer(DEBOUNCE_TIMEOUT).await;
            }

            while let Some(()) = updates.next().await {
                this.update(&mut cx, |this, cx| {
                    this.completion_id = completion_id;
                    cx.notify();
                })?;
            }
            Ok(())
        });
    }

    fn cycle(
        &mut self,
        _buffer: Model<Buffer>,
        _cursor_position: Anchor,
        _direction: Direction,
        _cx: &mut ModelContext<Self>,
    ) {
        // todo!("cycling")
    }

    fn accept(&mut self, _cx: &mut ModelContext<Self>) {
        self.pending_refresh = Task::ready(Ok(()));
        self.completion_id = None;
    }

    fn discard(&mut self, _cx: &mut ModelContext<Self>) {
        self.pending_refresh = Task::ready(Ok(()));
        self.completion_id = None;
    }

    fn active_completion_text<'a>(
        &'a self,
        buffer: &Model<Buffer>,
        cursor_position: Anchor,
        cx: &'a AppContext,
    ) -> Option<&'a str> {
        let completion_id = self.completion_id?;
        let buffer = buffer.read(cx);
        let cursor_offset = cursor_position.to_offset(buffer);
        let completion = Supermaven::get(cx).completion(completion_id)?;

        let mut completion_range = completion.range.to_offset(buffer);

        let prefix_len = common_prefix(
            buffer.chars_for_range(completion_range.clone()),
            completion.text.chars(),
        );
        completion_range.start += prefix_len;
        let suffix_len = common_prefix(
            buffer.reversed_chars_for_range(completion_range.clone()),
            completion.text[prefix_len..].chars().rev(),
        );
        completion_range.end = completion_range.end.saturating_sub(suffix_len);

        let completion_text = &completion.text[prefix_len..completion.text.len() - suffix_len];
        if completion_range.is_empty()
            && completion_range.start == cursor_offset
            && !completion_text.trim().is_empty()
        {
            Some(completion_text)
        } else {
            None
        }
    }
}

fn common_prefix<T1: Iterator<Item = char>, T2: Iterator<Item = char>>(a: T1, b: T2) -> usize {
    a.zip(b)
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum()
}
