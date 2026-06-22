use super::style::{StyledLine, StyledSegment, UiStyle, render_styled_line};

pub(super) fn input_prompt_label(last_execution_count: Option<u32>) -> String {
    match last_execution_count {
        Some(count) => format!("In [{}]", count.saturating_add(1)),
        None => "In [1]".to_string(),
    }
}

pub(super) fn input_prompt(execution_count: Option<u32>) -> String {
    execution_count
        .map(|count| format!("In [{count}]: "))
        .unwrap_or_else(|| "In [?]: ".to_string())
}

pub(super) fn continuation_prompt(input_prompt: &str) -> String {
    format!("{:>width$}", "...: ", width = input_prompt.len())
}

pub(super) fn output_prompt(execution_count: Option<u32>) -> String {
    execution_count
        .map(|count| format!("Out[{count}]"))
        .unwrap_or_else(|| "Out[?]".to_string())
}

pub(super) fn styled_input_prompt(prompt: &str) -> String {
    render_styled_line(&StyledLine::new(vec![StyledSegment::semantic(
        prompt,
        UiStyle::InputPrompt,
    )]))
}

pub(super) fn styled_output_prompt(prompt: &str) -> String {
    render_styled_line(&StyledLine::new(vec![StyledSegment::semantic(
        prompt,
        UiStyle::OutputPrompt,
    )]))
}

#[cfg(test)]
mod tests {
    use super::{continuation_prompt, input_prompt, input_prompt_label, output_prompt};

    #[test]
    fn builds_input_prompt_label_from_last_execution_count() {
        assert_eq!(input_prompt_label(None), "In [1]");
        assert_eq!(input_prompt_label(Some(2)), "In [3]");
    }

    #[test]
    fn builds_input_and_continuation_prompts() {
        let prompt = input_prompt(Some(12));

        assert_eq!(prompt, "In [12]: ");
        assert_eq!(continuation_prompt(&prompt), "    ...: ");
    }

    #[test]
    fn builds_output_prompts() {
        assert_eq!(output_prompt(Some(3)), "Out[3]");
        assert_eq!(output_prompt(None), "Out[?]");
    }
}
