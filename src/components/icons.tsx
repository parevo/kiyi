// One icon set (Lucide) everywhere, at consistent sizes, so the app reads as a single product.
import {
  AlertTriangle,
  ArrowDownUp,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Code2,
  Columns3,
  Database,
  Eye,
  Filter,
  Home,
  KeyRound,
  Link2,
  Loader2,
  type LucideProps,
  MoreHorizontal,
  PanelRight,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  Square,
  Table2,
  Trash2,
  X,
  Lock,
  Play,
  Copy,
  Undo2,
} from "lucide-react";

type P = LucideProps & { size?: number };
const make =
  (Icon: React.ComponentType<LucideProps>) =>
  ({ size = 16, strokeWidth = 1.75, ...rest }: P) => <Icon size={size} strokeWidth={strokeWidth} aria-hidden {...rest} />;

export const PlusIcon = make(Plus);
export const CloseIcon = make(X);
export const ChevronIcon = make(ChevronRight);
export const ChevronLeftIcon = make(ChevronLeft);
export const ChevronDownIcon = make(ChevronDown);
export const TableIcon = make(Table2);
export const ViewIcon = make(Eye);
export const DatabaseIcon = make(Database);
export const PlayIcon = make(Play);
export const StopIcon = make(Square);
export const RefreshIcon = make(RefreshCw);
export const MoreIcon = make(MoreHorizontal);
export const CheckIcon = make(Check);
export const AlertIcon = make(AlertTriangle);
export const LockIcon = make(Lock);
export const SearchIcon = make(Search);
export const PanelIcon = make(PanelRight);
export const SettingsIcon = make(Settings);
export const CodeIcon = make(Code2);
export const HomeIcon = make(Home);
export const FilterIcon = make(Filter);
export const SortIcon = make(ArrowDownUp);
export const SparklesIcon = make(Sparkles);
export const KeyIcon = make(KeyRound);
export const LinkIcon = make(Link2);
export const EditIcon = make(Pencil);
export const TrashIcon = make(Trash2);
export const ColumnsIcon = make(Columns3);
export const ArrowIcon = make(ArrowRight);
export const CopyIcon = make(Copy);
export const UndoIcon = make(Undo2);

export const Spinner = ({ size = 14 }: { size?: number }) => (
  <Loader2 size={size} strokeWidth={2} aria-label="Loading" style={{ animation: "spin 0.8s linear infinite" }} />
);
