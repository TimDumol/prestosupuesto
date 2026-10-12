import { AxisBottom, AxisLeft } from '@visx/axis';
import { curveLinear } from '@visx/curve';
import { localPoint } from '@visx/event';
import { GridRows } from '@visx/grid';
import { ParentSize } from '@visx/responsive';
import { scaleBand, scaleLinear, scaleTime } from '@visx/scale';
import { Bar, Circle, LinePath } from '@visx/shape';
import { defaultStyles, useTooltip, useTooltipInPortal } from '@visx/tooltip';
import { useEffect, useRef } from 'react';
import type { CSSProperties, KeyboardEvent, MouseEvent } from 'react';

export type MonthlySpendingDatum = { month: string; amount: number };
export type BudgetTimelineDatum = { month: string; categories: { name: string; amount: number }[] };
export type AccountHistoryDatum = { date: string; value: number; balanceEur: number | null };
export type AccountHistoryComparisonSeries = { key: string; label: string; points: AccountHistoryDatum[] };

const accountHistoryColors = ['#28704c', '#3478a5', '#bd7040', '#8663a8', '#ad4a59', '#79852d', '#268b82', '#925f42'];
const budgetTimelineColors = ['#28704c', '#3478a5', '#bd7040', '#8663a8', '#ad4a59', '#79852d', '#268b82', '#925f42', '#b36b9d', '#526aa8'];
const chartTooltipStyle: CSSProperties = {
  ...defaultStyles,
  zIndex: 10,
  maxWidth: 280,
  padding: '9px 11px',
  border: '1px solid #dce5dc',
  borderRadius: 8,
  color: '#34443a',
  background: '#fff',
  boxShadow: '0 8px 24px rgba(35, 54, 42, .14)',
  fontFamily: 'DM Sans, sans-serif',
  fontSize: '1rem',
  lineHeight: 1.45,
  pointerEvents: 'none',
};

const axisLabelProps = () => ({
  fill: '#59685f',
  fontFamily: 'DM Sans, sans-serif',
  fontSize: '1rem',
});

function timestamp(date: string) {
  return Date.parse(date + 'T00:00:00Z');
}

function dateKey(value: Date | number | { valueOf(): number }) {
  return (value instanceof Date ? value : new Date(Number(value))).toISOString().slice(0, 10);
}

function timeScaleForDates(dates: string[], range: [number, number]) {
  const values = dates.map(timestamp);
  const first = Math.min(...values);
  const last = Math.max(...values);
  const halfDay = 12 * 60 * 60 * 1000;
  return scaleTime({
    domain: first === last ? [new Date(first - halfDay), new Date(last + halfDay)] : [new Date(first), new Date(last)],
    range,
  });
}

function pointFromEvent(event: MouseEvent<SVGElement>) {
  const svg = event.currentTarget.ownerSVGElement;
  return svg ? localPoint(svg, event) : null;
}

function budgetCategoryColor(index: number) {
  return budgetTimelineColors[index] ?? `hsl(${(index * 137.508) % 360} 44% 46%)`;
}

export function BudgetComposition({ data, categoryNames, formatCurrency, remaining = false }: {
  data: { name: string; amount: number | null }[];
  categoryNames: string[];
  formatCurrency: (value: number, code?: string) => string;
  remaining?: boolean;
}) {
  const known = data.filter((item): item is { name: string; amount: number } => item.amount !== null && Number.isFinite(item.amount));
  const positive = known.filter((item) => item.amount > 0).sort((a, b) => b.amount - a.amount || a.name.localeCompare(b.name));
  const deficits = known.filter((item) => item.amount < 0).sort((a, b) => a.amount - b.amount);
  const total = positive.reduce((sum, item) => sum + item.amount, 0);
  const net = known.reduce((sum, item) => sum + item.amount, 0);
  const color = (name: string) => budgetCategoryColor(Math.max(0, categoryNames.indexOf(name)));
  const percent = (amount: number) => {
    const share = amount / total * 100;
    return share < 0.1 ? '<0.1%' : share.toFixed(1) + '%';
  };

  return <div className="budget-composition">
    <div className="budget-composition-total"><span>{remaining ? 'Positive remaining' : 'Total allocated'}</span><strong>{formatCurrency(total)}</strong></div>
    {total > 0 ? <>
      <div className="budget-composition-stack" aria-hidden="true">
        {positive.map((item) => <span key={item.name} style={{ width: item.amount / total * 100 + '%', backgroundColor: color(item.name) }} title={item.name + ' · ' + formatCurrency(item.amount) + ' · ' + percent(item.amount)} />)}
      </div>
      <ul className="budget-composition-list" aria-label={remaining ? 'Share of positive remaining budget by category' : 'Share of monthly allocations by category'}>
        {positive.map((item) => <li key={item.name}>
          <div className="budget-composition-label"><i style={{ backgroundColor: color(item.name) }} /><span>{item.name}</span><strong>{formatCurrency(item.amount)}</strong><small>{percent(item.amount)}</small></div>
          <div className="budget-composition-track" aria-hidden="true"><span style={{ width: item.amount / total * 100 + '%', backgroundColor: color(item.name) }} /></div>
        </li>)}
      </ul>
    </> : <p className="budget-composition-note">{remaining ? 'No positive budget remaining in this month.' : 'No monthly allocations recorded for this month.'}</p>}
    {deficits.length > 0 ? <div className="budget-composition-deficits"><strong>{remaining ? 'Category deficits' : 'Negative allocations'}</strong>
      <ul>{deficits.map((item) => <li key={item.name}><span>{item.name}</span><strong className="value-negative">{formatCurrency(item.amount)}</strong></li>)}</ul>
      <div className="budget-composition-total"><span>{remaining ? 'Net remaining after deficits' : 'Net allocated'}</span><strong>{formatCurrency(net)}</strong></div>
      <p className="budget-composition-note">Percentages show shares of positive amounts; negative amounts are included in the net total.</p>
    </div> : null}
    {data.some((item) => item.amount === null || !Number.isFinite(item.amount)) ? <p className="budget-composition-note">Categories with unavailable amounts are excluded.</p> : null}
  </div>;
}

type BudgetTimelineTooltipData = {
  month: string;
  total: number;
  categories: { name: string; amount: number }[];
};

export function BudgetSpendingTimeline({
  data,
  activeMonth,
  onSelectMonth,
  formatMonth,
  formatCurrency,
  formatCompactCurrency,
}: {
  data: BudgetTimelineDatum[];
  activeMonth: string;
  onSelectMonth: (month: string) => void;
  formatMonth: (month: string, short?: boolean) => string;
  formatCurrency: (value: number, code?: string) => string;
  formatCompactCurrency: (value: number, code: string) => string;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const { tooltipData, tooltipLeft, tooltipTop, tooltipOpen, showTooltip, hideTooltip } = useTooltip<BudgetTimelineTooltipData>();
  const { containerRef, TooltipInPortal } = useTooltipInPortal({ detectBounds: true, scroll: true });
  const categories = Array.from(new Set(data.flatMap((item) => item.categories.map((category) => category.name)))).sort((left, right) => left.localeCompare(right));
  const categoryColors = new Map(categories.map((name, index) => [name, budgetCategoryColor(index)]));
  const height = 350;
  const margin = { top: 18, right: 22, bottom: 48, left: 72 };
  const minWidth = Math.max(680, data.length * 64);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollLeft = scrollRef.current.scrollWidth;
  }, [data.length]);

  return <>
    <div className="budget-timeline-legend" aria-label="Spending categories">
      {categories.map((name) => <span key={name}><i style={{ backgroundColor: categoryColors.get(name) }} />{name}</span>)}
    </div>
    <div className="budget-timeline-scroll" ref={scrollRef}>
      <ParentSize className="budget-timeline-canvas" style={{ minWidth, height }} initialSize={{ width: minWidth, height }}>
        {({ width }) => {
          if (width < 10 || !data.length) return null;
          const plotWidth = Math.max(1, width - margin.left - margin.right);
          const plotHeight = height - margin.top - margin.bottom;
          const xScale = scaleBand({ domain: data.map((item) => item.month), range: [0, plotWidth], padding: 0.22 });
          const totals = data.map((item) => item.categories.reduce((sum, category) => sum + Math.max(0, category.amount), 0));
          const highest = Math.max(1, ...totals);
          const yScale = scaleLinear({ domain: [0, highest], range: [plotHeight, 0], nice: 4 });
          const tickMonths = data.filter((_, index) => index % 3 === 0).map((item) => item.month);
          if (data.length && tickMonths[tickMonths.length - 1] !== data[data.length - 1].month) tickMonths.push(data[data.length - 1].month);
          const tooltipForMonth = (item: BudgetTimelineDatum): BudgetTimelineTooltipData => ({
            month: item.month,
            total: item.categories.reduce((sum, category) => sum + Math.max(0, category.amount), 0),
            categories: item.categories.filter((category) => category.amount > 0).sort((left, right) => right.amount - left.amount),
          });
          const showMonthTooltip = (event: MouseEvent<SVGRectElement>, item: BudgetTimelineDatum) => {
            const location = pointFromEvent(event);
            const monthIndex = data.findIndex((month) => month.month === item.month);
            const barX = xScale(item.month) ?? 0;
            const stackTotal = totals[monthIndex] ?? 0;
            showTooltip({
              tooltipData: tooltipForMonth(item),
              tooltipLeft: location?.x ?? margin.left + barX + xScale.bandwidth() / 2,
              tooltipTop: location?.y ?? margin.top + yScale(stackTotal),
            });
          };
          const selectOnKey = (event: KeyboardEvent<SVGRectElement>, month: string) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onSelectMonth(month);
            }
          };

          return <>
            <svg ref={containerRef} width={width} height={height} role="group" aria-label="Monthly spending by budget category">
              <GridRows className="history-grid-line" top={margin.top} left={margin.left} scale={yScale} width={plotWidth} numTicks={4} stroke="#e9eee7" strokeWidth={1} />
              <AxisLeft
                scale={yScale}
                left={margin.left}
                top={margin.top}
                numTicks={4}
                hideAxisLine
                hideTicks
                tickFormat={(value) => formatCompactCurrency(Number(value), 'EUR')}
                tickLabelProps={axisLabelProps}
              />
              <AxisBottom
                scale={xScale}
                left={margin.left}
                top={margin.top + plotHeight}
                tickValues={tickMonths}
                hideAxisLine
                hideTicks
                tickFormat={(month) => {
                  const label = formatMonth(String(month), true);
                  const [monthName, year] = label.split(' ');
                  return year ? monthName + " '" + year.slice(-2) : label;
                }}
                tickLabelProps={axisLabelProps}
              />
              {data.map((item, monthIndex) => {
                let accumulated = 0;
                const barX = xScale(item.month) ?? 0;
                const barWidth = xScale.bandwidth();
                const segments = categories.map((name) => {
                  const amount = Math.max(0, item.categories.find((category) => category.name === name)?.amount ?? 0);
                  const start = accumulated;
                  accumulated += amount;
                  return { name, amount, start, end: accumulated };
                });
                const stackTotal = totals[monthIndex] ?? 0;
                return <g key={item.month}>
                  {segments.filter((segment) => segment.amount > 0).map((segment) => {
                    const top = yScale(segment.end);
                    const bottom = yScale(segment.start);
                    const color = categoryColors.get(segment.name) ?? '#28704c';
                    return <Bar
                      key={segment.name}
                      x={margin.left + barX}
                      y={margin.top + top}
                      width={barWidth}
                      height={Math.max(0, bottom - top)}
                      fill={color}
                      pointerEvents="none"
                    />;
                  })}
                  <Bar
                    className="budget-timeline-hit-area"
                    x={margin.left + barX}
                    y={margin.top + (stackTotal > 0 ? yScale(stackTotal) : plotHeight - 4)}
                    width={barWidth}
                    height={Math.max(4, plotHeight - yScale(stackTotal))}
                    fill="transparent"
                    stroke={item.month === activeMonth ? '#174b3b' : 'transparent'}
                    strokeWidth={item.month === activeMonth ? 2 : 0}
                    pointerEvents="all"
                    role="button"
                    tabIndex={0}
                    aria-label={formatMonth(item.month) + ', ' + formatCurrency(stackTotal) + ' spent. Select this month.'}
                    onClick={() => onSelectMonth(item.month)}
                    onKeyDown={(event) => selectOnKey(event, item.month)}
                    onMouseEnter={(event) => showMonthTooltip(event, item)}
                    onMouseMove={(event) => showMonthTooltip(event, item)}
                    onMouseLeave={hideTooltip}
                    onFocus={() => showTooltip({
                      tooltipData: tooltipForMonth(item),
                      tooltipLeft: margin.left + barX + barWidth / 2,
                      tooltipTop: margin.top + yScale(stackTotal),
                    })}
                    onBlur={hideTooltip}
                    onTouchStart={(event) => {
                      const svg = event.currentTarget.ownerSVGElement;
                      if (!svg) return;
                      const location = localPoint(svg, event);
                      showTooltip({
                        tooltipData: tooltipForMonth(item),
                        tooltipLeft: location?.x,
                        tooltipTop: location?.y,
                      });
                    }}
                  />
                </g>;
              })}
            </svg>
            {tooltipOpen && tooltipData ? <TooltipInPortal
              left={tooltipLeft}
              top={tooltipTop}
              style={{ ...chartTooltipStyle, maxWidth: 340, maxHeight: 'min(65vh, 360px)', overflowY: 'auto' }}
            >
              <strong>{formatMonth(tooltipData.month)}</strong><br />Total · {formatCurrency(tooltipData.total)}
              {tooltipData.categories.map((category) => <div className="chart-tooltip-series" key={category.name}>
                <i style={{ backgroundColor: categoryColors.get(category.name) }} />{category.name}<strong>{formatCurrency(category.amount)}</strong>
              </div>)}
              {!tooltipData.categories.length ? <div>No category spending recorded</div> : null}
            </TooltipInPortal> : null}
          </>;
        }}
      </ParentSize>
    </div>
  </>;
}

export function MonthlySpendingChart({
  data,
  activeMonth,
  onSelectMonth,
  formatMonth,
  formatCurrency,
  formatCompactCurrency,
}: {
  data: MonthlySpendingDatum[];
  activeMonth: string;
  onSelectMonth: (month: string) => void;
  formatMonth: (month: string, short?: boolean) => string;
  formatCurrency: (value: number, code?: string) => string;
  formatCompactCurrency: (value: number, code: string) => string;
}) {
  const { tooltipData, tooltipLeft, tooltipTop, tooltipOpen, showTooltip, hideTooltip } = useTooltip<MonthlySpendingDatum>();
  const { containerRef, TooltipInPortal } = useTooltipInPortal({ detectBounds: true, scroll: true });
  const height = 176;
  const margin = { top: 12, right: 8, bottom: 30, left: 62 };

  return <div className="trend-chart" aria-label="Monthly spending trend">
    <ParentSize className="trend-chart-canvas" initialSize={{ width: 720, height }}>
      {({ width }) => {
        if (width < 10 || !data.length) return null;
        const plotWidth = Math.max(1, width - margin.left - margin.right);
        const plotHeight = height - margin.top - margin.bottom;
        const xScale = scaleBand({
          domain: data.map((item) => item.month),
          range: [0, plotWidth],
          padding: 0.34,
        });
        const highest = Math.max(1, ...data.map((item) => item.amount));
        const yScale = scaleLinear({ domain: [0, highest], range: [plotHeight, 0], nice: 3 });
        const handlePointer = (event: MouseEvent<SVGRectElement>, item: MonthlySpendingDatum) => {
          const point = pointFromEvent(event);
          showTooltip({
            tooltipData: item,
            tooltipLeft: point?.x ?? margin.left + (xScale(item.month) ?? 0) + xScale.bandwidth() / 2,
            tooltipTop: point?.y ?? margin.top + yScale(item.amount),
          });
        };
        const selectOnKey = (event: KeyboardEvent<SVGRectElement>, month: string) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            onSelectMonth(month);
          }
        };

        return <>
          <svg ref={containerRef} width={width} height={height} role="group" aria-label="Monthly spending trend chart">
            <GridRows
              className="history-grid-line"
              top={margin.top}
              left={margin.left}
              scale={yScale}
              width={plotWidth}
              numTicks={3}
              stroke="#e9eee7"
              strokeWidth={1}
            />
            <AxisLeft
              scale={yScale}
              left={margin.left}
              top={margin.top}
              numTicks={3}
              hideAxisLine
              hideTicks
              tickFormat={(value) => formatCompactCurrency(Number(value), 'EUR')}
              tickLabelProps={axisLabelProps}
            />
            <AxisBottom
              scale={xScale}
              left={margin.left}
              top={margin.top + plotHeight}
              hideAxisLine
              hideTicks
              tickFormat={(month) => formatMonth(String(month), true).split(' ')[0]}
              tickLabelProps={axisLabelProps}
            />
            {data.map((item) => {
              const bandX = xScale(item.month) ?? 0;
              const barWidth = Math.min(xScale.bandwidth(), 35);
              const barX = bandX + (xScale.bandwidth() - barWidth) / 2;
              const barY = yScale(item.amount);
              const barHeight = Math.max(4, plotHeight - barY);
              const barTop = margin.top + plotHeight - barHeight;
              const label = formatMonth(item.month) + ': ' + formatCurrency(item.amount);
              return <Bar
                key={item.month}
                className={'trend-visx-bar' + (item.month === activeMonth ? ' trend-visx-bar-active' : '')}
                x={margin.left + barX}
                y={barTop}
                width={barWidth}
                height={barHeight}
                rx={4}
                fill={item.month === activeMonth ? '#174b3b' : '#b8cdbb'}
                role="button"
                tabIndex={0}
                aria-label={label + '. Select this month.'}
                onClick={() => onSelectMonth(item.month)}
                onKeyDown={(event) => selectOnKey(event, item.month)}
                onMouseEnter={(event) => handlePointer(event, item)}
                onMouseMove={(event) => handlePointer(event, item)}
                onMouseLeave={hideTooltip}
              onFocus={() => showTooltip({
                tooltipData: item,
                  tooltipLeft: margin.left + barX + barWidth / 2,
                tooltipTop: barTop,
                })}
                onBlur={hideTooltip}
                onTouchStart={(event) => {
                  const svg = event.currentTarget.ownerSVGElement;
                  if (!svg) return;
                  const point = localPoint(svg, event);
                  showTooltip({ tooltipData: item, tooltipLeft: point?.x, tooltipTop: point?.y });
                }}
              />;
            })}
          </svg>
          {tooltipOpen && tooltipData ? <TooltipInPortal left={tooltipLeft} top={tooltipTop} style={chartTooltipStyle}>
            <strong>{formatMonth(tooltipData.month)}</strong><br />{formatCurrency(tooltipData.amount)} spent
          </TooltipInPortal> : null}
        </>;
      }}
    </ParentSize>
  </div>;
}

export function AccountHistoryChart({
  points,
  label,
  code,
  formatDate,
  formatCurrency,
  formatCompactCurrency,
}: {
  points: AccountHistoryDatum[];
  label: string;
  code: string;
  formatDate: (date: string) => string;
  formatCurrency: (value: number, code?: string) => string;
  formatCompactCurrency: (value: number, code: string) => string;
}) {
  const sortedPoints = [...points].sort((left, right) => left.date.localeCompare(right.date));
  const { tooltipData, tooltipLeft, tooltipTop, tooltipOpen, showTooltip, hideTooltip } = useTooltip<AccountHistoryDatum>();
  const { containerRef, TooltipInPortal } = useTooltipInPortal({ detectBounds: true, scroll: true });
  const height = 300;
  const margin = { top: 20, right: 22, bottom: 40, left: 82 };

  return <div className="history-chart-wrap">
    <ParentSize className="history-chart-canvas" style={{ height }} initialSize={{ width: 920, height }}>
      {({ width }) => {
        if (width < 10 || !sortedPoints.length) return null;
        const plotWidth = Math.max(1, width - margin.left - margin.right);
        const plotHeight = height - margin.top - margin.bottom;
        const xScale = timeScaleForDates(sortedPoints.map((point) => point.date), [0, plotWidth]);
        const values = sortedPoints.map((point) => point.value);
        const low = Math.min(...values);
        const high = Math.max(...values);
        const padding = high === low ? Math.max(Math.abs(high) * 0.08, 1) : (high - low) * 0.12;
        const yScale = scaleLinear({ domain: [low - padding, high + padding], range: [plotHeight, 0], nice: 4 });
        const pointX = (point: AccountHistoryDatum) => xScale(new Date(timestamp(point.date))) ?? 0;
        const pointY = (point: AccountHistoryDatum) => yScale(point.value);
        const pointAtIndex = (index: number) => sortedPoints[index];
        const nearestPointIndex = (localX: number) => sortedPoints.reduce((nearest, point, index) => {
          const candidateX = pointX(point);
          const nearestX = pointX(sortedPoints[nearest]);
          return Math.abs(candidateX - localX) < Math.abs(nearestX - localX) ? index : nearest;
        }, 0);
        const showPointAt = (index: number, left: number, top: number) => {
          const point = pointAtIndex(index);
          showTooltip({ tooltipData: point, tooltipLeft: left, tooltipTop: top });
        };
        const handlePointer = (event: MouseEvent<SVGRectElement>) => {
          const location = pointFromEvent(event);
          if (!location) return;
          const localX = Math.max(0, Math.min(plotWidth, location.x - margin.left));
          showPointAt(nearestPointIndex(localX), location.x, location.y);
        };
        const handleKeyDown = (event: KeyboardEvent<SVGRectElement>) => {
          if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
          event.preventDefault();
          const currentIndex = tooltipData ? sortedPoints.findIndex((point) => point.date === tooltipData.date) : 0;
          const index = event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? sortedPoints.length - 1
              : Math.max(0, Math.min(sortedPoints.length - 1, currentIndex + (event.key === 'ArrowRight' ? 1 : -1)));
          const point = pointAtIndex(index);
          showPointAt(index, margin.left + pointX(point), margin.top + pointY(point));
        };
        const activePointIndex = tooltipData ? sortedPoints.findIndex((point) => point.date === tooltipData.date) : -1;
        const activePoint = activePointIndex >= 0 ? sortedPoints[activePointIndex] : null;

        return <>
          <svg ref={containerRef} width={width} height={height} role="group" aria-label={label + ' balance history chart'}>
            <GridRows className="history-grid-line" top={margin.top} left={margin.left} scale={yScale} width={plotWidth} numTicks={4} stroke="#e9eee7" strokeWidth={1} />
            <AxisLeft
              scale={yScale}
              left={margin.left}
              top={margin.top}
              numTicks={4}
              hideAxisLine
              hideTicks
              tickFormat={(value) => formatCompactCurrency(Number(value), code)}
              tickLabelProps={axisLabelProps}
            />
            <AxisBottom
              scale={xScale}
              left={margin.left}
              top={margin.top + plotHeight}
              numTicks={3}
              hideAxisLine
              hideTicks
              tickFormat={(value) => formatDate(dateKey(value))}
              tickLabelProps={axisLabelProps}
            />
            <LinePath
              data={sortedPoints}
              x={(point) => margin.left + pointX(point)}
              y={(point) => margin.top + pointY(point)}
              curve={curveLinear}
              fill="none"
              className="history-line"
            />
            {sortedPoints.map((point) => {
              return <g key={point.date}>
                <Circle cx={margin.left + pointX(point)} cy={margin.top + pointY(point)} r={4} className="history-point" pointerEvents="none" />
              </g>;
            })}
            {tooltipOpen && activePoint ? <g pointerEvents="none">
              <line x1={margin.left + pointX(activePoint)} x2={margin.left + pointX(activePoint)} y1={margin.top} y2={margin.top + plotHeight} className="history-hover-line" />
              <Circle cx={margin.left + pointX(activePoint)} cy={margin.top + pointY(activePoint)} r={6} className="history-hover-point" />
            </g> : null}
            <Bar
              x={margin.left}
              y={margin.top}
              width={plotWidth}
              height={plotHeight}
              fill="transparent"
              pointerEvents="all"
              role="slider"
              tabIndex={0}
              aria-label={label + ' balance history. Use the left and right arrow keys to inspect checkpoint dates.'}
              aria-valuemin={0}
              aria-valuemax={sortedPoints.length - 1}
              aria-valuenow={activePointIndex >= 0 ? activePointIndex : 0}
              aria-valuetext={activePoint ? formatDate(activePoint.date) + ', ' + formatCurrency(activePoint.value, code) : formatDate(sortedPoints[0].date)}
              onMouseMove={handlePointer}
              onMouseLeave={hideTooltip}
              onFocus={() => showPointAt(0, margin.left + pointX(sortedPoints[0]), margin.top + pointY(sortedPoints[0]))}
              onBlur={hideTooltip}
              onKeyDown={handleKeyDown}
              onTouchStart={(event) => {
                const svg = event.currentTarget.ownerSVGElement;
                if (!svg) return;
                const location = localPoint(svg, event);
                if (!location) return;
                const localX = Math.max(0, Math.min(plotWidth, location.x - margin.left));
                showPointAt(nearestPointIndex(localX), location.x, location.y);
              }}
            />
          </svg>
          {tooltipOpen && tooltipData ? <TooltipInPortal left={tooltipLeft} top={tooltipTop} style={chartTooltipStyle}>
            <strong>{formatDate(tooltipData.date)}</strong><br />{formatCurrency(tooltipData.value, code)}
            {tooltipData.balanceEur !== null && code !== 'EUR' ? <><br /><span>EUR equivalent: {formatCurrency(tooltipData.balanceEur, 'EUR')}</span></> : null}
          </TooltipInPortal> : null}
        </>;
      }}
    </ParentSize>
  </div>;
}

type ComparisonTooltipData = {
  date: string;
  items: { key: string; label: string; color: string; value: number }[];
};

export function AccountHistoryComparisonChart({
  series,
  formatDate,
  formatCompactCurrency,
  formatCurrency,
}: {
  series: AccountHistoryComparisonSeries[];
  formatDate: (date: string) => string;
  formatCompactCurrency: (value: number, code: string) => string;
  formatCurrency: (value: number, code?: string) => string;
}) {
  const dates = Array.from(new Set(series.flatMap((item) => item.points.map((point) => point.date)))).sort();
  const values = series.flatMap((item) => item.points.map((point) => point.value));
  const { tooltipData, tooltipLeft, tooltipTop, tooltipOpen, showTooltip, hideTooltip } = useTooltip<ComparisonTooltipData>();
  const { containerRef, TooltipInPortal } = useTooltipInPortal({ detectBounds: true, scroll: true });
  const height = 330;
  const margin = { top: 20, right: 22, bottom: 44, left: 82 };

  return <>
    <div className="history-comparison-legend" aria-label="Accounts shown in chart">
      {series.map((item, index) => <span key={item.key}><i style={{ backgroundColor: accountHistoryColors[index % accountHistoryColors.length] }} />{item.label}</span>)}
    </div>
    <div className="history-chart-wrap">
      <ParentSize className="history-chart-canvas history-comparison-chart-canvas" style={{ height }} initialSize={{ width: 920, height }}>
        {({ width }) => {
          if (width < 10 || !dates.length || !values.length) return null;
          const plotWidth = Math.max(1, width - margin.left - margin.right);
          const plotHeight = height - margin.top - margin.bottom;
          const dateScale = scaleBand({ domain: dates, range: [0, plotWidth], padding: 0.25 });
          const xScale = (date: Date) => (dateScale(dateKey(date)) ?? 0) + dateScale.bandwidth() / 2;
          const stacks = dates.map((date) => {
            let positive = 0;
            let negative = 0;
            const segments = series.flatMap((item, seriesIndex) => {
              const point = item.points.find((candidate) => candidate.date === date);
              if (!point) return [];
              const start = point.value >= 0 ? positive : negative;
              if (point.value >= 0) positive += point.value;
              else negative += point.value;
              return [{ key: item.key, seriesIndex, start, end: start + point.value }];
            });
            return { date, positive, negative, segments };
          });
          const low = Math.min(0, ...stacks.map((stack) => stack.negative));
          const high = Math.max(0, ...stacks.map((stack) => stack.positive));
          const padding = high === low ? Math.max(Math.abs(high) * 0.08, 1) : (high - low) * 0.12;
          const yScale = scaleLinear({ domain: [low < 0 ? low - padding : 0, high + padding], range: [plotHeight, 0], nice: 4 });
          const dateAtIndex = (index: number): ComparisonTooltipData => {
            const date = dates[index];
            return {
              date,
              items: series.flatMap((item, seriesIndex) => {
                const point = item.points.find((candidate) => candidate.date === date);
                return point ? [{
                  key: item.key,
                  label: item.label,
                  color: accountHistoryColors[seriesIndex % accountHistoryColors.length],
                  value: point.value,
                }] : [];
              }),
            };
          };
          const showDateTooltip = (data: ComparisonTooltipData, left: number, top: number) => {
            showTooltip({ tooltipData: data, tooltipLeft: left, tooltipTop: top });
          };
          const handlePointer = (event: MouseEvent<SVGRectElement>) => {
            const location = pointFromEvent(event);
            if (!location) return;
            const localX = Math.max(0, Math.min(plotWidth, location.x - margin.left));
            const index = dates.reduce((nearest, date, candidateIndex) => {
              const candidateX = xScale(new Date(timestamp(date))) ?? 0;
              const nearestX = xScale(new Date(timestamp(dates[nearest]))) ?? 0;
              return Math.abs(candidateX - localX) < Math.abs(nearestX - localX) ? candidateIndex : nearest;
            }, 0);
            showDateTooltip(dateAtIndex(index), location.x, location.y);
          };
          const handleKeyDown = (event: KeyboardEvent<SVGRectElement>) => {
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
            event.preventDefault();
            const currentDate = tooltipData?.date;
            const currentIndex = currentDate ? dates.indexOf(currentDate) : 0;
            const index = event.key === 'Home'
              ? 0
              : event.key === 'End'
                ? dates.length - 1
                : Math.max(0, Math.min(dates.length - 1, currentIndex + (event.key === 'ArrowRight' ? 1 : -1)));
            const data = dateAtIndex(index);
            showDateTooltip(data, margin.left + (xScale(new Date(timestamp(data.date))) ?? 0), margin.top + 10);
          };

          return <>
            <svg ref={containerRef} width={width} height={height} role="group" aria-label="Stacked account balances by checkpoint date in EUR">
              <GridRows className="history-grid-line" top={margin.top} left={margin.left} scale={yScale} width={plotWidth} numTicks={4} stroke="#e9eee7" strokeWidth={1} />
              <AxisLeft
                scale={yScale}
                left={margin.left}
                top={margin.top}
                numTicks={4}
                hideAxisLine
                hideTicks
                tickFormat={(value) => formatCompactCurrency(Number(value), 'EUR')}
                tickLabelProps={axisLabelProps}
              />
              <AxisBottom
                scale={dateScale}
                left={margin.left}
                top={margin.top + plotHeight}
                tickValues={dates.filter((_, index) => index % Math.max(1, Math.ceil(dates.length / 6)) === 0 || index === dates.length - 1)}
                hideAxisLine
                hideTicks
                tickFormat={(value) => formatDate(String(value))}
                tickLabelProps={axisLabelProps}
              />
              {stacks.map((stack) => <g key={stack.date}>
                {stack.segments.map((segment) => <Bar
                  key={segment.key}
                  x={margin.left + (dateScale(stack.date) ?? 0)}
                  y={margin.top + yScale(Math.max(segment.start, segment.end))}
                  width={dateScale.bandwidth()}
                  height={Math.abs(yScale(segment.start) - yScale(segment.end))}
                  fill={accountHistoryColors[segment.seriesIndex % accountHistoryColors.length]}
                  pointerEvents="none"
                />)}
              </g>)}
              {tooltipOpen && tooltipData ? <line
                x1={margin.left + (xScale(new Date(timestamp(tooltipData.date))) ?? 0)}
                x2={margin.left + (xScale(new Date(timestamp(tooltipData.date))) ?? 0)}
                y1={margin.top}
                y2={margin.top + plotHeight}
                className="history-hover-line"
                pointerEvents="none"
              /> : null}
              <Bar
                x={margin.left}
                y={margin.top}
                width={plotWidth}
                height={plotHeight}
                fill="transparent"
                pointerEvents="all"
                role="slider"
                tabIndex={0}
                aria-label="Account balance history. Use the left and right arrow keys to inspect checkpoint dates."
                aria-valuemin={0}
                aria-valuemax={dates.length - 1}
                aria-valuenow={tooltipData ? dates.indexOf(tooltipData.date) : 0}
                aria-valuetext={formatDate(tooltipData?.date ?? dates[0])}
                onMouseMove={handlePointer}
                onMouseLeave={hideTooltip}
                onFocus={() => {
                  const initial = dateAtIndex(0);
                  showDateTooltip(initial, margin.left + (xScale(new Date(timestamp(initial.date))) ?? 0), margin.top + 10);
                }}
                onBlur={hideTooltip}
                onKeyDown={handleKeyDown}
                onTouchStart={(event) => {
                  const svg = event.currentTarget.ownerSVGElement;
                  if (!svg) return;
                  const location = localPoint(svg, event);
                  if (!location) return;
                  const localX = Math.max(0, Math.min(plotWidth, location.x - margin.left));
                  const index = dates.reduce((nearest, date, candidateIndex) => {
                    const candidateX = xScale(new Date(timestamp(date))) ?? 0;
                    const nearestX = xScale(new Date(timestamp(dates[nearest]))) ?? 0;
                    return Math.abs(candidateX - localX) < Math.abs(nearestX - localX) ? candidateIndex : nearest;
                  }, 0);
                  showDateTooltip(dateAtIndex(index), location.x, location.y);
                }}
              />
            </svg>
            {tooltipOpen && tooltipData ? <TooltipInPortal left={tooltipLeft} top={tooltipTop} style={chartTooltipStyle}>
              <strong>{formatDate(tooltipData.date)}</strong>
              {tooltipData.items.map((item) => <div className="chart-tooltip-series" key={item.key}>
                <i style={{ backgroundColor: item.color }} />{item.label}<strong>{formatCurrency(item.value, 'EUR')}</strong>
              </div>)}
            </TooltipInPortal> : null}
          </>;
        }}
      </ParentSize>
    </div>
  </>;
}
