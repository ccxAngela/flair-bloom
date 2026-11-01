import './Kbd.css';

interface Props {
  children: string;
  label?: string;
  title?: string;
}

export default function Kbd({ children, label, title }: Props) {
  return (
    <kbd className="fb-kbd" title={title}>
      {children}
      {label && <span className="fb-kbd-label">{label}</span>}
    </kbd>
  );
}
