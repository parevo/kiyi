import type { ReactNode } from "react";
import {
  Button as AriaButton,
  type ButtonProps,
  Input,
  Label,
  Menu,
  MenuItem,
  MenuTrigger,
  Popover,
  Switch as AriaSwitch,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  TooltipTrigger,
} from "react-aria-components";
import s from "./ui.module.css";

const cx = (...c: (string | false | undefined | null)[]) => c.filter(Boolean).join(" ");

type Variant = "default" | "primary" | "ghost";

export function Button({ variant = "default", className, ...props }: ButtonProps & { variant?: Variant; className?: string }) {
  return (
    <AriaButton
      {...props}
      className={cx(s.button, variant === "primary" && s.primary, variant === "ghost" && s.ghost, className)}
    />
  );
}

export function IconButton({ label, shortcut, children, ...props }: ButtonProps & { label: string; shortcut?: string; children: ReactNode }) {
  return (
    <TooltipTrigger delay={500} closeDelay={0}>
      <AriaButton {...props} aria-label={label} className={cx(s.button, s.ghost, s.icon)}>
        {children}
      </AriaButton>
      <Tooltip className={s.tooltip} offset={6}>
        {label}
        {shortcut && <span className={s.kbd}>  {shortcut}</span>}
      </Tooltip>
    </TooltipTrigger>
  );
}

interface FieldProps {
  label: string;
  value: string;
  onChange(value: string): void;
  type?: string;
  placeholder?: string;
  mono?: boolean;
  autoFocus?: boolean;
  className?: string;
}

export function Field({ label, mono, className, placeholder, type, autoFocus, ...props }: FieldProps) {
  return (
    <TextField {...props} className={cx(s.field, className)} type={type} autoFocus={autoFocus}>
      <Label className={s.label}>{label}</Label>
      <Input className={cx(s.input, mono && s.mono)} placeholder={placeholder} spellCheck={false} autoCorrect="off" autoCapitalize="off" />
    </TextField>
  );
}

export function Segmented<T extends string>({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: T;
  onChange(value: T): void;
  options: { value: T; label: ReactNode }[];
}) {
  return (
    <div className={s.field}>
      <span className={s.label}>{label}</span>
      <ToggleButtonGroup
        aria-label={label}
        className={s.segmented}
        selectionMode="single"
        disallowEmptySelection
        selectedKeys={[value]}
        onSelectionChange={(keys) => {
          const next = [...keys][0];
          if (next) onChange(next as T);
        }}
      >
        {options.map((o) => (
          <ToggleButton key={o.value} id={o.value} className={s.segment}>
            {o.label}
          </ToggleButton>
        ))}
      </ToggleButtonGroup>
    </div>
  );
}

export function Switch({ isSelected, onChange, children }: { isSelected: boolean; onChange(v: boolean): void; children: ReactNode }) {
  return (
    <AriaSwitch isSelected={isSelected} onChange={onChange} className={s.switch}>
      <span className={s.track} />
      {children}
    </AriaSwitch>
  );
}

export interface MenuAction {
  id: string;
  label: string;
  danger?: boolean;
  onAction(): void;
}

export function ActionMenu({ trigger, actions }: { trigger: ReactNode; actions: MenuAction[] }) {
  return (
    <MenuTrigger>
      {trigger}
      <Popover className={s.popover} placement="bottom end" offset={4}>
        <Menu onAction={(key) => actions.find((a) => a.id === key)?.onAction()}>
          {actions.map((a) => (
            <MenuItem key={a.id} id={a.id} className={s.menuItem} data-danger={a.danger || undefined}>
              {a.label}
            </MenuItem>
          ))}
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}

export const kbd = s.kbd;
