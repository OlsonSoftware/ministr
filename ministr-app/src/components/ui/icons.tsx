/**
 * The ministr icon family — a single indirection over **Iconoir**
 * (`iconoir-react`, MIT, 1600+ glyphs on a strict 24×24 grid).
 *
 * Iconoir's distinctive 1.5px hairline stroke is the house "tell": thinner and
 * more characterful than the ubiquitous lucide/heroicons outline, yet crisp and
 * premium-coherent with the Liquid-Glass command-deck tier.
 *
 * ── Why this module exists ──────────────────────────────────────────────────
 * Every surface imports its icons from HERE, never from a library directly.
 * The icon vocabulary (the names below) is the app's own; the right column is
 * whichever Iconoir glyph currently realizes it. Swapping the underlying icon
 * library — or re-pointing a single glyph — is a one-file change, and no view
 * has to be touched. The names intentionally mirror the prior vocabulary so
 * call sites read unchanged.
 *
 * Stroke width: Iconoir's glyphs default to a distinctive 1.5px hairline but
 * honor a `strokeWidth` prop (it passes through to the SVG and the paths
 * inherit it), so the existing call-site `strokeWidth={…}` values keep working.
 * The house "tell" is the glyph geometry, not a forced stroke.
 */
export {
  Activity,
  WarningCircle as AlertCircle,
  WarningHexagon as AlertOctagon,
  WarningTriangle as AlertTriangle,
  ArrowDown,
  ArrowLeft,
  DataTransferBoth as ArrowLeftRight,
  ArrowRight,
  ArrowUpRight,
  Bookmark,
  Bookmark as BookmarkPlus,
  Box,
  Packages as Boxes,
  CodeBrackets as Braces,
  DataTransferBoth as Cable,
  Check,
  CheckCircle as CheckCircle2,
  NavArrowDown as ChevronDown,
  NavArrowRight as ChevronRight,
  WarningCircle as CircleAlert,
  Circle as CircleDashed,
  UserCircle as CircleUser,
  Clock,
  Cloud,
  CloudXmark as CloudOff,
  Code as Code2,
  KeyCommand as Command,
  Compass,
  Copy,
  Reply as CornerDownLeft,
  Cpu,
  Database,
  OpenInWindow as ExternalLink,
  Eye,
  EmptyPage as File,
  CodeBracketsSquare as FileCode,
  CodeBracketsSquare as FileCode2,
  Link as FileSymlink,
  Page as FileText,
  FireFlame as Flame,
  Flask as FlaskConical,
  Folder as FolderOpen,
  FolderPlus,
  DashboardSpeed as Gauge,
  GitBranch,
  GitCompare as GitCompareArrows,
  GitFork,
  Globe,
  HardDrive,
  Hashtag as Hash,
  ClockRotateRight as History,
  InfoCircle as Info,
  KeyCommand as Keyboard,
  Key as KeyRound,
  MultiplePages as Layers,
  BookStack as Library,
  LightBulb as Lightbulb,
  Link as Link2,
  FilterList as ListFilter,
  List as ListTree,
  RefreshDouble as Loader,
  RefreshDouble as Loader2,
  Lock,
  Message as MessageSquare,
  ChatPlusIn as MessageSquarePlus,
  Computer as MonitorSmartphone,
  HalfMoon as Moon,
  Network,
  Package as PackageOpen,
  SidebarExpand as PanelRight,
  Pause,
  EditPencil as PenLine,
  PhoneIncome as PhoneIncoming,
  PhoneOutcome as PhoneOutgoing,
  Pin,
  Play,
  EvPlug as Plug,
  Plus,
  Wrench as Power,
  Quote,
  AntennaSignal as Radio,
  Refresh as RefreshCw,
  Repeat,
  Rocket,
  Refresh as RotateCw,
  Scanning as ScanSearch,
  Scissor as Scissors,
  JournalPage as ScrollText,
  Search,
  Server,
  Settings,
  ShieldAlert,
  ShieldCheck,
  Collapse as Shrink,
  ControlSlider as SlidersHorizontal,
  Sparks as Sparkles,
  Leaf as Sprout,
  Healthcare as Stethoscope,
  SunLight as Sun,
  Brightness as SunMoon,
  Terminal,
  Terminal as TerminalSquare,
  Trash as Trash2,
  PineTree as TreePine,
  GraphDown as TrendingDown,
  WarningTriangle as TriangleAlert,
  EvPlugXmark as Unplug,
  Group as Users,
  ShareAndroid as Waypoints,
  RssFeed as Webhook,
  NetworkRight as Workflow,
  Xmark as X,
  Flash as Zap,
} from "iconoir-react";

import type { ComponentType, SVGProps } from "react";

/**
 * The shape every ministr icon satisfies: an SVG component that accepts the
 * usual `className` / `strokeWidth` props. Use this wherever an icon is passed
 * as a value (`icon={…}` / `icon: IconComponent`).
 */
export type IconComponent = ComponentType<
  SVGProps<SVGSVGElement> & { strokeWidth?: number | string }
>;

/**
 * Back-compat alias for the prior `LucideIcon` type name, so existing
 * `import { type LucideIcon }` call sites keep working unchanged.
 */
export type LucideIcon = IconComponent;
