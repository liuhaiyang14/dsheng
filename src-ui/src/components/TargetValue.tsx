import { formatMetricValue, type MetricRowView } from '../api';

export function hasConfirmedTarget(metric: MetricRowView): boolean {
  return metric.target_value != null && metric.target_basis?.kind === 'explicit_target' && !metric.target_basis.is_reference;
}

export function targetKindLabel(metric: MetricRowView): string {
  if (hasConfirmedTarget(metric)) return '目标';
  return metric.target_basis && metric.target_basis.kind !== 'unconfirmed_legacy' ? '参考值' : '基准值';
}

export function targetBasisLabel(metric: MetricRowView): string {
  if (metric.target_value == null) return '未设置';
  const basis = metric.target_basis;
  if (!basis || basis.kind === 'unconfirmed_legacy') return '来源待确认';
  const period = basis.from && basis.to ? ` · ${basis.from} 至 ${basis.to}` : '';
  return `${basis.label}${!hasConfirmedTarget(metric) ? ' · 参考，待确认目标' : ''}${period}`;
}

export function TargetValue({ metric }: { metric: MetricRowView }) {
  return <>
    <span>{formatMetricValue(metric.target_value, metric.unit, metric.unit_signature)}</span>
    <small className="target-basis">{targetBasisLabel(metric)}</small>
  </>;
}
