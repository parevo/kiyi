import type { ReactNode } from "react";
import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { CloseIcon } from "./icons";
import { IconButton } from "./ui";
import s from "./Sheet.module.css";

/** A panel that slides in from the right for focused tasks: insert a row, edit a column. */
export function Sheet({
  isOpen,
  onClose,
  title,
  subtitle,
  width = 460,
  footer,
  children,
}: {
  isOpen: boolean;
  onClose(): void;
  title: ReactNode;
  subtitle?: ReactNode;
  width?: number;
  /** Buttons; the first child is pushed to the left (e.g. a destructive action). */
  footer?: ReactNode;
  children: ReactNode;
}) {
  return (
    <ModalOverlay isOpen={isOpen} onOpenChange={(open) => !open && onClose()} isDismissable className={s.overlay}>
      <Modal className={s.sheet} style={{ width }}>
        <Dialog className={s.dialog}>
          <div className={s.header}>
            <div>
              <Heading slot="title" className={s.title}>
                {title}
              </Heading>
              {subtitle && <p className={s.subtitle}>{subtitle}</p>}
            </div>
            <IconButton label="Close" onPress={onClose}>
              <CloseIcon />
            </IconButton>
          </div>
          <div className={s.body}>{children}</div>
          {footer && <div className={s.footer}>{footer}</div>}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
