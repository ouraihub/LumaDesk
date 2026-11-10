use super::*;

impl ModelEditor {
    pub(super) fn prepare_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut values = vec![
            ("id".into(), self.value.id.clone()),
            (
                "name".into(),
                self.value.settings.name.clone().unwrap_or_default(),
            ),
            (
                "group".into(),
                self.value.settings.group_name.clone().unwrap_or_default(),
            ),
            ("context".into(), text(self.value.settings.context_window)),
            ("input".into(), text(self.value.settings.max_input)),
            ("output".into(), text(self.value.settings.max_output)),
        ];
        if let Some(pricing) = &self.value.settings.pricing {
            for (index, price) in pricing.tiers.iter().enumerate() {
                for (field, value) in [
                    ("start", price.min_input_tokens.to_string()),
                    ("input", price.input.to_string()),
                    ("output", price.output.to_string()),
                    ("read", text(price.cache_read)),
                    ("write", text(price.cache_write)),
                ] {
                    values.push((format!("pricing.{index}.{field}"), value));
                }
            }
        }
        self.inputs
            .retain(|key, _| values.iter().any(|(name, _)| name == key));
        for (key, initial) in values {
            if self.inputs.contains_key(&key) {
                continue;
            }
            let initial = self
                .state
                .text
                .entry(key.clone())
                .or_insert(initial)
                .clone();
            let placeholder = match key.as_str() {
                "name" => "留空使用模型 ID",
                "group" => "留空自动分组",
                "context" => "自动使用来源参考或默认容量",
                "input" => "留空不单独限制输入",
                "output" => "留空使用默认输出预算",
                _ => "",
            };
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(initial)
                    .placeholder(placeholder)
            });
            let name = key.clone();
            let subscription = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.apply_text(&name, value);
                    cx.notify();
                }
            });
            self.inputs.insert(key, (input, subscription));
        }
    }
    pub(super) fn reset(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.state.clear(prefix);
        self.inputs.retain(|key, _| !key.starts_with(prefix));
        self.prepare_inputs(window, cx);
    }
    pub(super) fn apply_text(&mut self, key: &str, value: String) {
        self.state.text.insert(key.into(), value.clone());
        self.state.errors.remove(key);
        let result = match key {
            "id" => {
                if !self.existing {
                    self.value.id = value;
                }
                Ok(())
            }
            "name" => {
                self.value.settings.name = (!value.trim().is_empty()).then_some(value);
                Ok(())
            }
            "group" => {
                self.value.settings.group_name = (!value.trim().is_empty()).then_some(value);
                Ok(())
            }
            "context" => parse(
                &value,
                &mut self.value.settings.context_window,
                false,
                |v| v > 0,
            ),
            "input" => parse(&value, &mut self.value.settings.max_input, false, |v| v > 0),
            "output" => parse(&value, &mut self.value.settings.max_output, false, |v| {
                v > 0
            }),
            _ => {
                let result = key
                    .strip_prefix("pricing.")
                    .and_then(|s| s.split_once('.'))
                    .and_then(|(index, field)| {
                        index.parse::<usize>().ok().map(|index| (index, field))
                    });
                if let Some((index, field)) = result {
                    if let Some(price) = self
                        .value
                        .settings
                        .pricing
                        .as_mut()
                        .and_then(|p| p.tiers.get_mut(index))
                    {
                        match field {
                            "start" => {
                                let mut target = Some(price.min_input_tokens);
                                let result = parse(&value, &mut target, true, |_| true);
                                if let Some(v) = target {
                                    price.min_input_tokens = v;
                                }
                                result
                            }
                            "input" | "output" => {
                                let target = if field == "input" {
                                    &mut price.input
                                } else {
                                    &mut price.output
                                };
                                let mut amount = Some(*target);
                                let result = parse(&value, &mut amount, true, valid_price);
                                if let Some(v) = amount {
                                    *target = v;
                                }
                                result
                            }
                            "read" => parse(&value, &mut price.cache_read, false, valid_price),
                            "write" => parse(&value, &mut price.cache_write, false, valid_price),
                            _ => Ok(()),
                        }
                    } else {
                        Ok(())
                    }
                } else {
                    Ok(())
                }
            }
        };
        if result.is_err() {
            self.state.errors.insert(
                key.into(),
                format!("{key}：请输入有效数字（预算/容量为正整数，价格为 0–1000000）"),
            );
        }
    }
}
