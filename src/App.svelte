<script>
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { check } from '@tauri-apps/plugin-updater';
  import { relaunch } from '@tauri-apps/plugin-process';
  import Icon from './lib/Icon.svelte';

  let report = $state(null);
  let activeView = $state('report');
  let activePage = $state(0);
  let activeTable = $state(0);
  let activeQuery = $state(0);
  let activeDaxQuery = $state(0);
  let activeColumn = $state(0);
  let activeVisual = $state(-1);
  let dataTable = $state(0);
  let tableData = $state(null);
  let dataLoading = $state(false);
  let dataError = $state('');
  let dataOffset = $state(0);
  let search = $state('');
  let loading = $state(false);
  let error = $state('');
  let dragging = $state(false);
  let entryContent = $state(null);
  let entryLoading = $state(false);
  let fieldDialog = $state(null);
  let visualExplanation = $state(null);
  let copied = $state('');
  let querySearch = $state('');
  let tableSearch = $state('');
  let columnSearch = $state('');
  let dataFilter = $state('');
  let sortColumn = $state(-1);
  let sortAscending = $state(true);
  let hideSystemTables = $state(true);
  let loadingMessage = $state('Reading package…');
  let reportZoom = $state(100);
  let reportFitMode = $state(true);
  let canvasWrap = $state(null);
  let hiddenVisuals = $state([]);
  let recent = $state(JSON.parse(localStorage.getItem('pbi-lens-recent') || '[]'));
  let availableUpdate = $state(null);
  let updateMenuOpen = $state(false);
  let updateStatus = $state('idle');
  let updateProgress = $state(null);
  let updateError = $state('');
  let daxRunLoading = $state(false);
  let daxRunError = $state('');
  let daxRunResult = $state(null);
  let daxRunQuery = $state('');
  let aasServerUrl = $state('');
  let aasCatalog = $state('');
  let aasTenantId = $state('');
  let aasClientId = $state('');
  let aasClientSecret = $state('');
  let aasRole = $state('');
  let aasCustomData = $state('');

  const UPDATE_CHECK_KEY = 'pbi-lens-update-last-check';
  const UPDATE_CHECK_INTERVAL = 24 * 60 * 60 * 1000;

  const nav = [
    ['report', 'chart', 'Report'],
    ['model', 'model', 'Model'],
    ['data', 'table', 'Data'],
    ['sources', 'database', 'Sources'],
    ['queries', 'search', 'Queries'],
    ['dax', 'play', 'Run DAX'],
    ['contents', 'files', 'Contents']
  ];

  function readableSize(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1048576).toFixed(1)} MB`;
  }

  function cleanName(value) {
    return String(value || '').replace(/^'|'$/g, '').replace(/\\u0027/g, "'");
  }

  async function openFile(path) {
    loading = true;
    loadingMessage = 'Reading package…';
    const progressTimer = setTimeout(() => loadingMessage = 'Decoding semantic model locally…', 1200);
    error = '';
    try {
      const data = path ? await invoke('open_report_path', { path }) : await invoke('choose_report');
      if (!data) return;
      report = data;
      const firstBusinessTable = data.tables.findIndex((table) => !table.is_hidden && !table.name.startsWith('LocalDateTable_') && !table.name.startsWith('DateTableTemplate_'));
      activeView = 'report';
      activePage = 0;
      activeTable = firstBusinessTable >= 0 ? firstBusinessTable : 0;
      activeQuery = 0;
      const defaultDaxQuery = data.dax_queries?.findIndex((query) => query.is_default) ?? -1;
      activeDaxQuery = defaultDaxQuery >= 0 ? defaultDaxQuery : 0;
      activeColumn = 0;
      activeVisual = -1;
      dataTable = firstBusinessTable >= 0 ? firstBusinessTable : 0;
      reportZoom = 100;
      tableData = null;
      dataOffset = 0;
      entryContent = null;
      fieldDialog = null;
      visualExplanation = null;
      querySearch = '';
      tableSearch = '';
      columnSearch = '';
      dataFilter = '';
      reportFitMode = true;
      hiddenVisuals = (data.pages[0]?.visuals || []).map((visual, index) => visual.is_hidden ? index : -1).filter((index) => index >= 0);
      aasServerUrl ||= data.aas_connection?.server_url || '';
      aasCatalog ||= data.aas_connection?.catalog || '';
      daxRunResult = null;
      daxRunError = '';
      const entry = { path: data.path, name: data.name, size: data.size };
      recent = [entry, ...recent.filter((r) => r.path !== entry.path)].slice(0, 6);
      localStorage.setItem('pbi-lens-recent', JSON.stringify(recent));
    } catch (e) {
      error = String(e).replace(/^Error: /, '');
    } finally {
      clearTimeout(progressTimer);
      loading = false;
    }
  }

  async function inspectEntry(entry) {
    entryLoading = true;
    entryContent = null;
    try {
      entryContent = await invoke('read_package_entry', { path: report.path, entryName: entry.name });
    } catch (e) {
      entryContent = { name: entry.name, kind: 'Error', content: String(e), truncated: false };
    } finally {
      entryLoading = false;
    }
  }

  async function copyText(value, label) {
    await navigator.clipboard.writeText(value || '');
    copied = label;
    setTimeout(() => { if (copied === label) copied = ''; }, 1400);
  }

  async function loadTable(index = dataTable, offset = 0) {
    const table = report?.tables?.[index];
    if (!table) return;
    dataTable = index;
    dataOffset = offset;
    tableData = null;
    dataError = '';
    dataLoading = true;
    try {
      tableData = await invoke('read_table_rows', { path: report.path, tableName: table.name, offset, limit: 100 });
      report.tables[index].row_count = tableData.total;
    } catch (e) {
      dataError = String(e).replace(/^Error: /, '');
    } finally {
      dataLoading = false;
    }
  }

  function selectView(view) {
    activeView = view;
    if (view === 'data' && !tableData && !dataLoading) loadTable(dataTable, 0);
  }

  function copyTablePage() {
    if (!tableData) return;
    const quote = (value) => `"${String(value ?? '').replaceAll('"', '""')}"`;
    const csv = [tableData.columns, ...tableData.rows].map((row) => row.map(quote).join(',')).join('\n');
    copyText(csv, 'table-page');
  }

  function goToQuery(name) {
    const index = report?.queries?.findIndex((query) => query.name === name) ?? -1;
    if (index >= 0) {
      activeQuery = index;
      querySearch = '';
      activeView = 'sources';
      setTimeout(() => document.querySelector('.query-list button.active')?.scrollIntoView({ block: 'nearest' }));
    }
  }

  function fieldTarget(value) {
    const raw = cleanName(value).replaceAll("'", '').trim();
    const lower = raw.toLowerCase();
    const tableMatches = (report?.tables || []).map((table, tableIndex) => ({ table, tableIndex })).filter(({ table }) => {
      const name = table.name.toLowerCase();
      return lower === name || lower.includes(`${name}[`) || lower.includes(`${name}.`) || lower.includes(`[${name}]`);
    }).sort((a, b) => b.table.name.length - a.table.name.length);
    let tableIndex = tableMatches[0]?.tableIndex ?? -1;
    let columnIndex = -1;
    if (tableIndex >= 0) {
      const columns = report.tables[tableIndex].columns;
      columnIndex = columns.findIndex((column) => {
        const name = column.name.toLowerCase();
        return lower === name || lower.includes(`[${name}]`) || lower.endsWith(`.${name}`);
      });
    } else {
      const exactColumns = (report?.tables || []).flatMap((table, candidateTable) => table.columns.map((column, candidateColumn) => ({ tableIndex: candidateTable, columnIndex: candidateColumn, name: column.name.toLowerCase(), hidden: table.is_hidden || column.is_hidden }))).filter((column) => column.name === lower).sort((a, b) => Number(a.hidden) - Number(b.hidden));
      if (exactColumns.length) {
        tableIndex = exactColumns[0].tableIndex;
        columnIndex = exactColumns[0].columnIndex;
      }
    }
    const tableName = tableIndex >= 0 ? report.tables[tableIndex].name : '';
    const queryIndex = (report?.queries || []).findIndex((query) => query.name.toLowerCase() === (tableName || raw).toLowerCase());
    return { tableIndex, columnIndex, queryIndex };
  }

  function openFieldInModel(field) {
    const target = fieldTarget(field);
    if (target.tableIndex < 0) return;
    const table = report.tables[target.tableIndex];
    const column = target.columnIndex >= 0 ? table.columns[target.columnIndex] : null;
    const expression = column?.expression || '';
    fieldDialog = {
      kind: expression ? 'DAX EXPRESSION' : 'MODEL FIELD',
      title: column ? `${table.name}.${column.name}` : table.name,
      subtitle: column ? `${column.kind || 'Field'} · ${column.data_type || 'No data type'}${column.cardinality != null ? ` · ${column.cardinality} distinct` : ''}` : `${table.columns.length} fields`,
      content: expression || [`Table: ${table.name}`, column ? `Field: ${column.name}` : '', column ? `Kind: ${column.kind || '—'}` : '', column ? `Data type: ${column.data_type || '—'}` : '', column?.cardinality != null ? `Cardinality: ${column.cardinality}` : '', column ? `Hidden: ${column.is_hidden ? 'Yes' : 'No'}` : ''].filter(Boolean).join('\n'),
      language: expression ? 'DAX' : 'MODEL'
    };
  }

  function openFieldQuery(field) {
    const target = fieldTarget(field);
    if (target.queryIndex < 0) return;
    const query = report.queries[target.queryIndex];
    fieldDialog = {
      kind: 'POWER QUERY',
      title: `${query.name}.m`,
      subtitle: [...(query.connectors || []), ...(query.dependencies || []).map((name) => `uses ${name}`)].join(' · ') || 'Packaged M definition',
      content: query.formula || 'No M expression was exposed for this query.',
      language: 'M'
    };
  }

  function openVisualQuery(visual) {
    const payload = {};
    if (visual?.prototype_query && Object.keys(visual.prototype_query).length) payload.prototypeQuery = visual.prototype_query;
    if (visual?.semantic_query && Object.keys(visual.semantic_query).length) payload.semanticQueryDataShape = visual.semantic_query;
    if (visual?.data_transforms && Object.keys(visual.data_transforms).length) payload.dataTransforms = visual.data_transforms;
    fieldDialog = {
      kind: 'VISUAL SEMANTIC QUERY',
      title: visual?.title || visual?.visual_type_label || 'Visual query',
      subtitle: `${visual?.aggregations?.length || 0} aggregations · the semantic query drives this visual; prototypeQuery stores its authoring bindings`,
      content: Object.keys(payload).length ? JSON.stringify(payload, null, 2) : 'No semantic query was packaged for this visual.',
      language: 'JSON'
    };
  }

  function openBookmark(bookmark) {
    fieldDialog = {
      kind: 'BOOKMARK STATE',
      title: bookmark.name,
      subtitle: `${bookmark.hidden_visual_count} hidden visuals · ${bookmark.filter_count} filter nodes · active page ${bookmark.active_page || 'unknown'}`,
      content: JSON.stringify(bookmark.state, null, 2),
      language: 'JSON'
    };
  }

  function openCachedWhere(filter) {
    fieldDialog = {
      kind: 'CACHED MERGED WHERE',
      title: 'Power BI resolved query filter',
      subtitle: filter.note || 'The filter scopes Power BI merged when this visual last ran.',
      content: filter.expression || 'No resolved filter expression was packaged.',
      language: 'FILTER'
    };
  }

  function openInteraction(interaction, otherLabel) {
    fieldDialog = {
      kind: 'EDIT INTERACTION',
      title: `${interaction.behavior}: ${otherLabel}`,
      subtitle: 'The complete page-level interaction stored by Power BI.',
      content: [
        `Behavior: ${interaction.behavior}`,
        `Type: ${interaction.interaction_type}`,
        `Source visual: ${interaction.source}`,
        `Target visual: ${interaction.target}`
      ].join('\n'),
      language: 'INTERACTION'
    };
  }

  function visualExplanationLabel(visual) {
    const type = (visual?.visual_type || '').toLowerCase();
    if (type.includes('kpi') || type.includes('card')) return 'EXPLAIN KPI';
    if (type.includes('table') || type.includes('matrix')) return 'EXPLAIN TABLE';
    if (type.includes('slicer')) return 'EXPLAIN SLICER';
    if (['chart', 'bar', 'line', 'area', 'pie', 'donut', 'scatter', 'treemap', 'funnel', 'waterfall', 'ribbon', 'map', 'gauge'].some((kind) => type.includes(kind))) return 'EXPLAIN CHART';
    return 'EXPLAIN VISUAL';
  }

  function explainVisual(visual) {
    const page = report.pages[activePage];
    const calculations = [];
    const seenFields = new Set();
    for (const aggregation of visual.aggregations || []) {
      const field = aggregation.field || aggregation.native_name || 'Unknown field';
      seenFields.add(field);
      calculations.push({
        name: aggregation.display_name || aggregation.native_name || field,
        detail: `${aggregation.function_name} · function code ${aggregation.function_code} · ${field}`,
        origin: 'Implicit visual aggregation',
        confidence: 'exact',
        expression: ''
      });
    }
    for (const field of visual.fields || []) {
      if (seenFields.has(field)) continue;
      seenFields.add(field);
      const target = fieldTarget(field);
      const table = target.tableIndex >= 0 ? report.tables[target.tableIndex] : null;
      const column = table && target.columnIndex >= 0 ? table.columns[target.columnIndex] : null;
      const kind = (column?.kind || '').toLowerCase();
      let origin = 'Packaged field reference';
      let confidence = 'exact';
      if (kind.includes('report measure')) origin = 'Report-level DAX measure';
      else if (kind.includes('measure') && column?.expression) origin = 'Model measure with decoded DAX';
      else if (kind.includes('measure')) origin = 'Model-side measure reference';
      else if (column) origin = 'Model column';
      else {
        origin = 'Unresolved field reference';
        confidence = 'unknown';
      }
      calculations.push({
        name: cleanName(field),
        detail: column ? `${table.name}[${column.name}] · ${column.kind || 'field'}` : cleanName(field),
        origin,
        confidence,
        expression: column?.expression || ''
      });
    }

    const scopedFilters = [
      ...(report.report_filters || []),
      ...(page?.filters || []),
      ...(visual.filters || []),
      ...(visual.slicer_selections || [])
    ];
    const resolvedFilters = visual.resolved_filters || [];
    const interactions = (page?.interactions || [])
      .filter((item) => item.source === visual.id || item.target === visual.id)
      .map((item) => {
        const otherId = item.source === visual.id ? item.target : item.source;
        const other = page?.visuals.find((candidate) => candidate.id === otherId);
        return {
          behavior: item.behavior,
          direction: item.source === visual.id ? 'To' : 'From',
          other: other?.title || other?.visual_type_label || otherId,
          type: item.interaction_type
        };
      });
    const bookmark = visual.bookmark_target
      ? report.bookmarks?.find((item) => item.id === visual.bookmark_target)
      : null;
    const behaviors = [
      { label: 'Initial visibility', value: visual.is_hidden ? 'Hidden' : 'Visible', confidence: 'exact' },
      ...(page?.is_hidden ? [{ label: 'Page visibility', value: 'Hidden page', confidence: 'exact' }] : []),
      ...(page?.is_drillthrough ? [{ label: 'Page purpose', value: 'Drillthrough target', confidence: 'exact' }] : []),
      ...(bookmark ? [{ label: 'Bookmark action', value: bookmark.name, confidence: 'exact' }] : visual.bookmark_target ? [{ label: 'Bookmark action', value: visual.bookmark_target, confidence: 'exact' }] : []),
      ...(visual.sync_group ? [{ label: 'Synced slicer group', value: `${visual.sync_group.group_name || 'Unnamed'} · filter changes ${visual.sync_group.filter_changes ? 'on' : 'off'}`, confidence: 'exact' }] : [])
    ];

    const unknowns = ['Business meaning cannot be proven from field and measure names alone.'];
    if (!resolvedFilters.length) unknowns.push('No cached merged query is packaged, so the final filter intersection must be reconstructed from individual scopes.');
    else unknowns.push('The cached merged query is Power BI’s last saved execution and may be stale if Desktop did not rerun the visual.');
    if (calculations.some((item) => item.origin === 'Model-side measure reference' && !item.expression)) unknowns.push('At least one measure is defined in the remote model; its DAX definition requires live-model metadata.');
    if (!calculations.length) unknowns.push('No conventional field or calculation binding was decoded for this visual type.');
    unknowns.push('Current values and contributing rows require a live AAS query.');

    const primary = calculations[0];
    const summary = primary
      ? `This ${visual.visual_type_label.toLowerCase()} uses ${primary.name}. ${primary.origin}. ${resolvedFilters.length ? `Power BI packaged ${resolvedFilters.length} merged filter condition${resolvedFilters.length === 1 ? '' : 's'} for its last execution.` : 'No final cached query was packaged.'}`
      : `This ${visual.visual_type_label.toLowerCase()} has no conventional calculation binding that PBI Lens can safely summarize.`;

    visualExplanation = {
      title: visual.title || visual.visual_type_label || 'Visual',
      type: visual.visual_type_label,
      summary,
      calculations,
      resolvedFilters,
      scopedFilters,
      interactions,
      behaviors,
      unknowns
    };
  }

  function explanationMarkdown(explanation) {
    const lines = [
      `# ${explanation.title}`,
      '',
      `Type: ${explanation.type}`,
      '',
      explanation.summary,
      '',
      '## Calculations',
      ...explanation.calculations.map((item) => `- [${item.confidence.toUpperCase()}] ${item.name}: ${item.origin} — ${item.detail}`),
      '',
      '## Cached merged filters',
      ...(explanation.resolvedFilters.length ? explanation.resolvedFilters.map((item) => `- [EXACT] ${item.expression}`) : ['- [UNKNOWN] No cached merged query was packaged.']),
      '',
      '## Scoped filters',
      ...(explanation.scopedFilters.length ? explanation.scopedFilters.map((item) => `- [EXACT] ${item.scope}: ${item.expression}`) : ['- No individual report, page, visual, or slicer filters were decoded.']),
      '',
      '## Behavior',
      ...explanation.behaviors.map((item) => `- [${item.confidence.toUpperCase()}] ${item.label}: ${item.value}`),
      ...explanation.interactions.map((item) => `- [EXACT] ${item.behavior} ${item.direction.toLowerCase()} ${item.other} (type ${item.type})`),
      '',
      '## Unknown or requires live model',
      ...explanation.unknowns.map((item) => `- ${item}`)
    ];
    return lines.join('\n');
  }

  function openDaxRunner(query) {
    if (query?.expression) daxRunQuery = query.expression;
    aasServerUrl ||= report?.aas_connection?.server_url || '';
    aasCatalog ||= report?.aas_connection?.catalog || '';
    daxRunResult = null;
    daxRunError = '';
    activeView = 'dax';
  }

  async function runDax() {
    daxRunLoading = true;
    daxRunError = '';
    daxRunResult = null;
    try {
      daxRunResult = await invoke('run_dax_query', {
        request: {
          serverUrl: aasServerUrl,
          catalog: aasCatalog,
          tenantId: aasTenantId,
          clientId: aasClientId,
          clientSecret: aasClientSecret,
          role: aasRole,
          customData: aasCustomData,
          query: daxRunQuery
        }
      });
    } catch (e) {
      daxRunError = String(e).replace(/^Error: /, '');
    } finally {
      daxRunLoading = false;
    }
  }

  function visibleTables() {
    const needle = tableSearch.trim().toLowerCase();
    return (report?.tables || []).map((table, index) => ({ table, index })).filter(({ table }) => {
      const system = table.is_hidden || table.name.startsWith('LocalDateTable_') || table.name.startsWith('DateTableTemplate_');
      return (!hideSystemTables || !system) && (!needle || table.name.toLowerCase().includes(needle));
    });
  }

  function visibleColumns() {
    const columns = report?.tables?.[activeTable]?.columns || [];
    const needle = columnSearch.trim().toLowerCase();
    return columns.map((column, index) => ({ column, index })).filter(({ column }) => !needle || column.name.toLowerCase().includes(needle) || column.kind.toLowerCase().includes(needle));
  }

  function visibleRows() {
    if (!tableData) return [];
    const needle = dataFilter.trim().toLowerCase();
    const rows = needle ? tableData.rows.filter((row) => row.some((cell) => String(cell ?? '').toLowerCase().includes(needle))) : [...tableData.rows];
    if (sortColumn >= 0) rows.sort((a, b) => String(a[sortColumn] ?? '').localeCompare(String(b[sortColumn] ?? ''), undefined, { numeric: true }) * (sortAscending ? 1 : -1));
    return rows;
  }

  function visiblePageFields() {
    const internal = /localdatetable_|datetabletemplate_|\.variation\.|dsepassthru|templateidcolumnrole|^(v\d+|batchstart|batchend|rawscore|expectedhigh|expectedlow|expectedvalue)$/i;
    return [...new Set(report?.pages?.[activePage]?.visuals.flatMap((visual) => visual.fields) || [])]
      .map(cleanName)
      .filter((field) => field && !internal.test(field))
      .slice(0, 24);
  }

  function sortData(index) {
    if (sortColumn === index) sortAscending = !sortAscending;
    else { sortColumn = index; sortAscending = true; }
  }

  function updateCanvasSize() {
    if (!reportFitMode || !canvasWrap || !report?.pages?.[activePage]) return;
    const page = report.pages[activePage];
    const styles = getComputedStyle(canvasWrap);
    const horizontalPadding = parseFloat(styles.paddingLeft) + parseFloat(styles.paddingRight);
    const verticalPadding = parseFloat(styles.paddingTop) + parseFloat(styles.paddingBottom);
    const availableWidth = Math.max(1, canvasWrap.clientWidth - horizontalPadding);
    const availableHeight = Math.max(1, canvasWrap.clientHeight - verticalPadding);
    const width = page.width || 1280;
    const height = page.height || 720;
    reportZoom = Math.max(10, Math.min(200, Math.min(availableWidth / width, availableHeight / height) * 100));
  }

  function setReportZoom(value) {
    reportFitMode = false;
    reportZoom = Math.max(10, Math.min(200, value));
  }

  function fitReport() {
    reportFitMode = true;
    updateCanvasSize();
  }

  function toggleVisualVisibility(index) {
    hiddenVisuals = hiddenVisuals.includes(index)
      ? hiddenVisuals.filter((item) => item !== index)
      : [...hiddenVisuals, index];
    if (hiddenVisuals.includes(index) && activeVisual === index) activeVisual = -1;
  }

  function selectVisual(index) {
    if (hiddenVisuals.includes(index)) hiddenVisuals = hiddenVisuals.filter((item) => item !== index);
    activeVisual = index;
  }

  $effect(() => {
    report;
    activePage;
    canvasWrap;
    if (!canvasWrap) return;
    const observer = new ResizeObserver(updateCanvasSize);
    observer.observe(canvasWrap);
    setTimeout(updateCanvasSize);
    return () => observer.disconnect();
  });

  function startWindowDrag(event) {
    if (event.button !== 0 || event.target.closest('button, input, a')) return;
    event.preventDefault();
    invoke('start_window_drag');
  }

  async function checkForUpdate(force = false) {
    const lastCheck = Number(localStorage.getItem(UPDATE_CHECK_KEY) || 0);
    if (!force && Date.now() - lastCheck < UPDATE_CHECK_INTERVAL) return;
    updateStatus = 'checking';
    updateError = '';
    try {
      const candidate = await check();
      localStorage.setItem(UPDATE_CHECK_KEY, String(Date.now()));
      if (candidate?.available) {
        availableUpdate = candidate;
        updateStatus = 'available';
      } else {
        availableUpdate = null;
        updateStatus = 'current';
        updateMenuOpen = false;
      }
    } catch (e) {
      updateStatus = 'error';
      updateError = String(e).replace(/^Error: /, '');
    }
  }

  async function installUpdate() {
    if (!availableUpdate || updateStatus === 'downloading' || updateStatus === 'installing') return;
    updateStatus = 'downloading';
    updateProgress = null;
    let downloaded = 0;
    let total = 0;
    try {
      await availableUpdate.downloadAndInstall((event) => {
        if (event.event === 'Started') total = event.data.contentLength || 0;
        if (event.event === 'Progress') {
          downloaded += event.data.chunkLength || 0;
          updateProgress = total ? Math.min(100, Math.round(downloaded / total * 100)) : null;
        }
        if (event.event === 'Finished') updateStatus = 'installing';
      });
      await relaunch();
    } catch (e) {
      updateStatus = 'error';
      updateError = String(e).replace(/^Error: /, '');
    }
  }

  async function setupDrop() {
    await listen('tauri://drag-enter', () => dragging = true);
    await listen('tauri://drag-leave', () => dragging = false);
    await listen('tauri://drag-drop', (event) => {
      dragging = false;
      const path = event.payload?.paths?.find((p) => /\.(pbix|pbit)$/i.test(p));
      if (path) openFile(path);
    });
    await listen('pbi-lens://open', async (event) => {
      await invoke('take_pending_paths');
      const path = event.payload?.find((p) => /\.(pbix|pbit)$/i.test(p));
      if (path) openFile(path);
    });
    const pending = await invoke('take_pending_paths');
    if (pending?.[0]) openFile(pending[0]);
  }
  setupDrop();
  if (import.meta.env.VITE_PBI_LENS_PROD === 'true') setTimeout(() => checkForUpdate(), 3000);
  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && fieldDialog) {
      fieldDialog = null;
      return;
    }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'o') {
      event.preventDefault();
      openFile();
    }
  });

  const filteredEntries = $derived(report?.entries?.filter((e) => e.name.toLowerCase().includes(search.toLowerCase())) || []);
</script>

<svelte:head><title>{report ? `${report.name} — PBI Lens` : 'PBI Lens'}</title></svelte:head>

{#if !report}
  <main class:drop-active={dragging} class="welcome-shell">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="traffic-space" data-tauri-drag-region onmousedown={startWindowDrag}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header class="welcome-header" data-tauri-drag-region onmousedown={startWindowDrag}>
      <div class="brand"><span class="brand-mark"><i></i><i></i><i></i></span><span>PBI Lens</span></div>
      <div class="welcome-actions">
        <span class="privacy-pill"><span></span> Everything stays on your Mac</span>
        {#if availableUpdate}
          <div class="update-control">
            <button class="update-icon" class:busy={updateStatus === 'downloading' || updateStatus === 'installing'} onclick={() => updateMenuOpen = !updateMenuOpen} title={`Update ${availableUpdate.version} available`} aria-label="Show available update"><Icon name="download" size={16}/><i></i></button>
            {#if updateMenuOpen}
              <div class="update-popover"><strong>Update {availableUpdate.version}</strong><span>{updateStatus === 'downloading' ? `Downloading${updateProgress == null ? '…' : ` ${updateProgress}%`}` : updateStatus === 'installing' ? 'Installing…' : updateStatus === 'error' ? updateError : 'Ready when you are. Nothing installs automatically.'}</span><button onclick={installUpdate} disabled={updateStatus === 'downloading' || updateStatus === 'installing'}>{updateStatus === 'downloading' || updateStatus === 'installing' ? 'Please wait' : 'Update and restart'}</button></div>
            {/if}
          </div>
        {/if}
      </div>
    </header>
    <section class="welcome-content">
      <div class="eyebrow">POWER BI EXPLORER FOR MAC</div>
      <h1>See what’s inside.<br><em>Without Windows.</em></h1>
      <p class="lede">Open and inspect Power BI reports natively. Explore pages, visuals, models, connections, and every packaged file—fast and private.</p>
      <button class="primary-action" onclick={() => openFile()} disabled={loading}>
        <Icon name="folder" size={19} />
        {loading ? loadingMessage : 'Open a Power BI file'}
        <span class="shortcut">⌘ O</span>
      </button>
      <div class="drop-hint"><Icon name="upload" size={16}/><span>or drop a .pbix or .pbit anywhere</span></div>
      {#if error}<div class="error"><Icon name="warning" size={17}/>{error}</div>{/if}

      {#if recent.length}
        <div class="recent-block">
          <div class="section-label">RECENT</div>
          <div class="recent-list">
            {#each recent.slice(0, 3) as item}
              <button onclick={() => openFile(item.path)}>
                <span class="file-tile"><Icon name="chart" size={19}/></span>
                <span><strong>{item.name}</strong><small>{readableSize(item.size)} · {item.path}</small></span>
                <Icon name="chevron" size={15}/>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </section>
    <footer><span>Local-first by design</span><span>•</span><span>No account</span><span>•</span><span>No upload</span></footer>
    {#if dragging}<div class="drop-overlay"><div><Icon name="upload" size={34}/><strong>Drop to explore</strong><span>PBIX and PBIT files are supported</span></div></div>{/if}
  </main>
{:else}
  <main class="app-shell" class:drop-active={dragging}>
    <aside class="rail">
      <div class="rail-logo"><span class="brand-mark mini"><i></i><i></i><i></i></span></div>
      <nav>
        {#each nav as item}
          <button class:active={activeView === item[0]} onclick={() => selectView(item[0])} title={item[2]}><Icon name={item[1]} size={20}/><span>{item[2]}</span></button>
        {/each}
      </nav>
      <button
        class="rail-update"
        class:busy={updateStatus === 'checking' || updateStatus === 'downloading' || updateStatus === 'installing'}
        class:available={availableUpdate}
        onclick={() => checkForUpdate(true)}
        disabled={updateStatus === 'checking' || updateStatus === 'downloading' || updateStatus === 'installing'}
        title={updateStatus === 'checking' ? 'Checking for updates…' : updateStatus === 'current' ? 'PBI Lens is up to date' : updateStatus === 'error' ? `Update check failed: ${updateError}` : availableUpdate ? `Update ${availableUpdate.version} available` : 'Check for updates'}
      >
        <Icon name={updateStatus === 'current' ? 'check' : 'download'} size={20}/>
        <span>{updateStatus === 'checking' ? 'Checking' : updateStatus === 'current' ? 'Current' : updateStatus === 'error' ? 'Retry' : availableUpdate ? 'Update' : 'Updates'}</span>
        {#if availableUpdate}<i></i>{/if}
      </button>
    </aside>

    <section class="workspace">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <header class="topbar" data-tauri-drag-region onmousedown={startWindowDrag}>
        <div class="file-identity">
          <button class="icon-button" onclick={() => report = null} title="Back"><Icon name="back" size={18}/></button>
          <span class="file-icon"><Icon name="chart" size={18}/></span>
          <span><strong>{report.name}</strong><small>{readableSize(report.size)} · {report.kind}</small></span>
        </div>
        <div class="status"><span class="status-dot"></span>{report.deep_cache_hit ? 'Opened from local cache' : 'Parsed locally'} in {report.parse_ms} ms</div>
        {#if availableUpdate}
          <div class="update-control">
            <button class="update-icon" class:busy={updateStatus === 'downloading' || updateStatus === 'installing'} onclick={() => updateMenuOpen = !updateMenuOpen} title={`Update ${availableUpdate.version} available`} aria-label="Show available update"><Icon name="download" size={16}/><i></i></button>
            {#if updateMenuOpen}
              <div class="update-popover"><strong>Update {availableUpdate.version}</strong><span>{updateStatus === 'downloading' ? `Downloading${updateProgress == null ? '…' : ` ${updateProgress}%`}` : updateStatus === 'installing' ? 'Installing…' : updateStatus === 'error' ? updateError : 'Ready when you are. Nothing installs automatically.'}</span><button onclick={installUpdate} disabled={updateStatus === 'downloading' || updateStatus === 'installing'}>{updateStatus === 'downloading' || updateStatus === 'installing' ? 'Please wait' : 'Update and restart'}</button></div>
            {/if}
          </div>
        {/if}
        <button class="open-button" onclick={() => openFile()}><Icon name="folder" size={16}/> Open</button>
      </header>
      {#if report.deep_error}<div class="decode-warning"><Icon name="warning" size={14}/><span><strong>Model decoder warning</strong>{report.deep_error}</span></div>{/if}

      {#if activeView === 'report'}
        <div class="view-layout report-view">
          <aside class="sidepanel">
            <div class="panel-title"><span>PAGES</span><b>{report.pages.length}</b></div>
            <div class="page-list">
              {#each report.pages as page, i}
                <button class:active={activePage === i} onclick={() => { activePage = i; activeVisual = -1; hiddenVisuals = page.visuals.map((visual, index) => visual.is_hidden ? index : -1).filter((index) => index >= 0); reportFitMode = true; setTimeout(updateCanvasSize); }}>
                  <span class="page-thumb"><span></span><i></i><i></i></span>
                  <span><strong>{page.display_name}</strong><small>{page.visuals.length} visuals{page.is_drillthrough ? ' · drillthrough' : page.is_hidden ? ' · hidden' : ''}</small></span>
                </button>
              {/each}
            </div>
          </aside>
          <section class="canvas-area">
            <div class="view-heading">
              <div><div class="crumb">REPORT / PAGE {activePage + 1}</div><h2>{report.pages[activePage]?.display_name || 'Report'}</h2></div>
              <div class="zoom-controls"><span>{report.pages[activePage]?.width || 1280} × {report.pages[activePage]?.height || 720}</span><button onclick={() => setReportZoom(reportZoom - 10)} title="Zoom out">−</button><input type="range" min="10" max="200" step="1" value={reportZoom} oninput={(event) => setReportZoom(Number(event.currentTarget.value))} aria-label="Report zoom"/><b>{Math.round(reportZoom)}%</b><button onclick={() => setReportZoom(reportZoom + 10)} title="Zoom in">+</button><button class:active={reportFitMode} class="fit-button" onclick={fitReport}>FIT</button></div>
            </div>
            <div class="report-canvas-wrap" bind:this={canvasWrap}>
              <div class="canvas-stage" style={`width:${(report.pages[activePage]?.width || 1280) * reportZoom / 100}px;height:${(report.pages[activePage]?.height || 720) * reportZoom / 100}px`}>
              <div class="report-canvas" style={`width:${report.pages[activePage]?.width || 1280}px;height:${report.pages[activePage]?.height || 720}px;transform:scale(${reportZoom / 100})`}>
                {#if report.pages[activePage]?.visuals.length}
                  {#each report.pages[activePage].visuals as visual, visualIndex}
                    {#if visual.x_pct >= 0 && visual.y_pct >= 0 && visual.x_pct + visual.w_pct <= 100.75 && visual.y_pct + visual.h_pct <= 100.75}
                    {@const visualType = visual.visual_type.toLowerCase()}
                    <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
                    <article class="visual" class:selected={activeVisual === visualIndex} class:visual-hidden={hiddenVisuals.includes(visualIndex)} class:chrome-free={['actionbutton','textbox','image','basicshape'].some(t => visualType.includes(t)) || visualType === 'shape'} class:kpi-visual={visualType === 'kpi'} style={`left:${visual.x_pct}%;top:${visual.y_pct}%;width:${visual.w_pct}%;height:${visual.h_pct}%;z-index:${visual.z_index}`} onclick={() => selectVisual(visualIndex)} onkeydown={(event) => { if (event.key === 'Enter' || event.key === ' ') selectVisual(visualIndex); }} role="button" tabindex="0" title="Select visual to inspect">
                      {#if visual.title && visualType !== 'kpi'}<header><strong>{visual.title}</strong></header>{/if}
                      <div class={`visual-preview type-${visual.visual_type}`}>
                        {#if visualType === 'kpi'}
                          <div class="kpi-preview"><span>{visual.title || 'KPI'}</span><div><b>—</b><i aria-hidden="true"><em></em></i></div></div>
                        {:else if visualType.includes('multirowcard')}
                          <div class="multi-card-preview">{#each visual.fields.slice(0, 3) as field}<div><span>{cleanName(field)}</span><b>—</b></div>{/each}</div>
                        {:else if visualType.includes('card')}
                          <b>—</b><span>{cleanName(visual.fields[0] || 'Value')}</span>
                        {:else if visualType.includes('keydrivers') || visualType.includes('keyinfluencer')}
                          <div class="ai-preview"><div class="ai-kicker"><span>AI</span> KEY INFLUENCERS</div><strong>Factors that influence this result</strong><div class="influence-row"><i></i><div><b>{cleanName(visual.fields[0] || 'Analysis field')}</b><em>Bound report field</em></div><span>FIELD</span></div><div class="influence-row"><i></i><div><b>{cleanName(visual.fields[1] || 'Analysis field')}</b><em>Bound report field</em></div><span>FIELD</span></div></div>
                        {:else if visualType.includes('decomposition')}
                          <div class="tree-preview"><div><b>Result</b></div><i></i><section><span>{cleanName(visual.fields[0] || 'Category')}</span><span>{cleanName(visual.fields[1] || 'Segment')}</span><span>{cleanName(visual.fields[2] || 'Detail')}</span></section></div>
                        {:else if visualType.includes('qna')}
                          <div class="qna-preview"><span>Q&amp;A</span><strong>Ask a question about your data</strong><div>Try “show sales by category”</div></div>
                        {:else if visualType.includes('table') || visualType.includes('matrix')}
                          <div class="fake-table">{#each Array(18) as _}<i></i>{/each}</div>
                        {:else if visualType.includes('slicer')}
                          <div class="fake-slicer"><span>Select</span><Icon name="chevron" size={13}/></div>
                        {:else if visualType.includes('actionbutton')}
                          <div class="fake-action"><span>{visual.title || 'Action'}</span></div>
                        {:else if visualType.includes('textbox')}
                          <div class="fake-text">{#if visual.title}<strong>{visual.title}</strong>{:else}<i></i><i></i><i></i>{/if}</div>
                        {:else if visualType.includes('image')}
                          <div class="fake-image" aria-label="Image asset"></div>
                        {:else if visualType.includes('shape')}
                          <div class="fake-shape"></div>
                        {:else if visualType.includes('line') || visualType.includes('area')}
                          <div class="line-preview"><i></i><i></i><i></i><i></i><i></i></div>
                        {:else if visualType.includes('bar')}
                          <div class="horizontal-bars"><i style="width:68%"></i><i style="width:90%"></i><i style="width:52%"></i><i style="width:77%"></i></div>
                        {:else}
                          <div class="bars"><i style="height:38%"></i><i style="height:74%"></i><i style="height:55%"></i><i style="height:88%"></i><i style="height:64%"></i></div>
                        {/if}
                      </div>
                    </article>
                    {/if}
                  {/each}
                {:else}
                  <div class="empty-canvas"><Icon name="grid" size={30}/><strong>No visual definitions found</strong><span>The page metadata is present but has no classic visual containers.</span></div>
                {/if}
              </div>
              </div>
            </div>
          </section>
          <aside class="inspector">
            <div class="panel-title"><span>{activeVisual >= 0 ? 'VISUAL INSPECTOR' : 'REPORT INFO'}</span>{#if activeVisual >= 0}<button class="clear-selection" onclick={() => activeVisual = -1}>CLEAR</button>{/if}</div>
            {#if activeVisual >= 0 && report.pages[activePage]?.visuals[activeVisual]}
              {@const selectedVisual = report.pages[activePage].visuals[activeVisual]}
              <dl>
                <div><dt>Title</dt><dd>{selectedVisual.title || 'Untitled'}</dd></div>
                <div><dt>Type</dt><dd>{selectedVisual.visual_type_label}</dd></div>
                <div><dt>Position</dt><dd>{Math.round(selectedVisual.x_pct)}%, {Math.round(selectedVisual.y_pct)}%</dd></div>
                <div><dt>Size</dt><dd>{Math.round(selectedVisual.w_pct)}% × {Math.round(selectedVisual.h_pct)}%</dd></div>
                <div><dt>Layer</dt><dd>{selectedVisual.z_index}</dd></div>
                <div><dt>Default state</dt><dd>{selectedVisual.is_hidden ? 'Hidden' : 'Visible'}</dd></div>
                {#if selectedVisual.sync_group}<div><dt>Sync group</dt><dd>{selectedVisual.sync_group.group_name}</dd></div>{/if}
              </dl>
              <button class="inspector-action explain-action" onclick={() => explainVisual(selectedVisual)}>{visualExplanationLabel(selectedVisual)}</button>
              {#if selectedVisual.aggregations?.length}
                <div class="panel-title second"><span>AGGREGATIONS</span><b>{selectedVisual.aggregations.length}</b></div>
                <div class="filter-list aggregation-list">
                  {#each selectedVisual.aggregations as aggregation}
                    <div><strong>{aggregation.display_name || aggregation.native_name || aggregation.field}</strong><span>{aggregation.function_name} · function code {aggregation.function_code}</span><code>{aggregation.field}</code></div>
                  {/each}
                </div>
              {/if}
              {#if selectedVisual.resolved_filters?.length}
                <div class="panel-title second"><span>CACHED MERGED WHERE</span><b>{selectedVisual.resolved_filters.length}</b></div>
                <div class="filter-list resolved-filter-list">
                  {#each selectedVisual.resolved_filters as filter}
                    <button class="detail-card" onclick={() => openCachedWhere(filter)} title="Open complete cached filter"><strong>Power BI resolved query</strong><code>{filter.expression}</code><em>{filter.note}</em><small>CLICK TO EXPAND</small></button>
                  {/each}
                </div>
              {/if}
              {@const selectedInteractions = (report.pages[activePage]?.interactions || []).filter((item) => item.source === selectedVisual.id || item.target === selectedVisual.id)}
              {#if selectedInteractions.length}
                <div class="panel-title second"><span>EDIT INTERACTIONS</span><b>{selectedInteractions.length}</b></div>
                <div class="filter-list interaction-list">
                  {#each selectedInteractions as interaction}
                    {@const otherId = interaction.source === selectedVisual.id ? interaction.target : interaction.source}
                    {@const other = report.pages[activePage]?.visuals.find((visual) => visual.id === otherId)}
                    {@const otherLabel = other?.title || other?.visual_type_label || otherId}
                    <button class="detail-card" onclick={() => openInteraction(interaction, otherLabel)} title="Open complete edit interaction"><strong>{interaction.behavior}</strong><span>{interaction.source === selectedVisual.id ? 'To' : 'From'} {otherLabel}</span><code>type {interaction.interaction_type}</code><small>CLICK TO EXPAND</small></button>
                  {/each}
                </div>
              {/if}
              {@const effectiveFilters = [...(report.report_filters || []), ...(report.pages[activePage]?.filters || []), ...(selectedVisual.filters || []), ...(selectedVisual.slicer_selections || [])]}
              {#if effectiveFilters.length}
                <div class="panel-title second"><span>FILTER CONTEXT</span><b>{effectiveFilters.length}</b></div>
                <div class="filter-list">
                  {#each effectiveFilters as filter}
                    <div class:inactive={!filter.active}><strong>{filter.scope} · {filter.kind || 'Filter'}</strong><span>{filter.target || 'Selection'}</span><code>{filter.expression}</code>{#if filter.note}<em>{filter.note}</em>{/if}</div>
                  {/each}
                </div>
              {/if}
              {#if selectedVisual.column_labels?.length}
                <div class="panel-title second"><span>DISPLAY LABELS</span><b>{selectedVisual.column_labels.length}</b></div>
                <div class="filter-list label-list">{#each selectedVisual.column_labels as label}<div><strong>{label.display_name}</strong><code>{label.query_ref}</code></div>{/each}</div>
              {/if}
              {#if selectedVisual.bookmark_target}
                {@const bookmark = report.bookmarks?.find((item) => item.id === selectedVisual.bookmark_target)}
                <button class="inspector-action query-action" onclick={() => bookmark && openBookmark(bookmark)}>BOOKMARK → {bookmark?.name || selectedVisual.bookmark_target}</button>
              {/if}
              {#if (selectedVisual.prototype_query && Object.keys(selectedVisual.prototype_query).length) || (selectedVisual.semantic_query && Object.keys(selectedVisual.semantic_query).length)}
                <button class="inspector-action query-action" onclick={() => openVisualQuery(selectedVisual)}>VIEW SEMANTIC QUERY</button>
              {/if}
              <button class="inspector-action" onclick={() => copyText(JSON.stringify(selectedVisual, null, 2), 'visual')}>{copied === 'visual' ? 'COPIED' : 'COPY VISUAL METADATA'}</button>
              <button class="inspector-action subtle" onclick={() => toggleVisualVisibility(activeVisual)}>HIDE FROM PREVIEW</button>
            {:else}
              <dl>
                <div><dt>Format</dt><dd>{report.kind}</dd></div>
                <div><dt>Pages</dt><dd>{report.pages.length}</dd></div>
                <div><dt>Visuals</dt><dd>{report.visual_count}</dd></div>
                <div><dt>Model tables</dt><dd>{report.tables.length}</dd></div>
                <div><dt>Saved DAX queries</dt><dd>{report.dax_queries?.length || 0}</dd></div>
                <div><dt>Bookmarks</dt><dd>{report.bookmarks?.length || 0}</dd></div>
                <div><dt>Page interactions</dt><dd>{report.pages[activePage]?.interactions?.length || 0}</dd></div>
                <div><dt>Mobile layouts</dt><dd>{report.mobile_layout_count || 0}</dd></div>
                <div><dt>Package files</dt><dd>{report.entries.length}</dd></div>
              </dl>
              {#if report.pages[activePage]?.interactions?.length}
                <div class="panel-title second"><span>EDIT INTERACTIONS</span><b>{report.pages[activePage].interactions.length}</b></div>
                <div class="interaction-summary"><span>{report.pages[activePage].interactions.filter((item) => item.interaction_type === 1).length} filter</span><span>{report.pages[activePage].interactions.filter((item) => item.interaction_type === 3).length} disconnected</span></div>
              {/if}
              {#if report.bookmarks?.length}
                <div class="panel-title second"><span>BOOKMARKS</span><b>{report.bookmarks.length}</b></div>
                <div class="bookmark-list">{#each report.bookmarks as bookmark}<button onclick={() => openBookmark(bookmark)}><strong>{bookmark.name}</strong><span>{bookmark.hidden_visual_count} hidden · {bookmark.filter_count} filters</span></button>{/each}</div>
              {/if}
              {#if report.report_filters?.length}
                <div class="panel-title second"><span>REPORT FILTERS</span><b>{report.report_filters.length}</b></div>
                <div class="filter-list">
                  {#each report.report_filters as filter}
                    <div class:inactive={!filter.active}><strong>{filter.kind || 'Filter'}</strong><span>{filter.target}</span><code>{filter.expression}</code></div>
                  {/each}
                </div>
              {/if}
              {#if report.pages[activePage]?.filters?.length}
                <div class="panel-title second"><span>PAGE FILTERS</span><b>{report.pages[activePage].filters.length}</b></div>
                <div class="filter-list">
                  {#each report.pages[activePage].filters as filter}
                    <div class:inactive={!filter.active}><strong>{filter.kind || 'Filter'}</strong><span>{filter.target}</span><code>{filter.expression}</code></div>
                  {/each}
                </div>
              {/if}
            {/if}
            <div class="panel-title second"><span>{activeVisual >= 0 ? 'BOUND FIELDS' : 'PAGE FIELDS'}</span></div>
            {#if activeVisual >= 0}
              <div class="field-links">
                {#each report.pages[activePage]?.visuals[activeVisual]?.fields || [] as field}
                  {@const target = fieldTarget(field)}
                  <div><span title={cleanName(field)}>{cleanName(field)}</span><aside>{#if target.tableIndex >= 0}<button onclick={() => openFieldInModel(field)}>{target.columnIndex >= 0 && report.tables[target.tableIndex].columns[target.columnIndex]?.expression ? 'DAX' : 'MODEL'} →</button>{/if}{#if target.queryIndex >= 0}<button onclick={() => openFieldQuery(field)}>QUERY →</button>{/if}</aside></div>
                {:else}<p>No field bindings exposed.</p>{/each}
              </div>
            {:else}
              <div class="field-cloud">
                {#each visiblePageFields() as field}<span>{field}</span>{:else}<p>No field bindings exposed.</p>{/each}
              </div>
            {/if}
            <div class="panel-title second layers-title"><span>PAGE LAYERS</span>{#if hiddenVisuals.length}<button class="clear-selection" onclick={() => hiddenVisuals = []}>SHOW ALL</button>{/if}</div>
            <div class="layer-list">
              {#each report.pages[activePage]?.visuals || [] as visual, i}
                <div class:active={activeVisual === i} class:hidden={hiddenVisuals.includes(i)}>
                  <button class="layer-main" onclick={() => selectVisual(i)} title={visual.title || visual.visual_type_label}><Icon name={visual.visual_type.toLowerCase().includes('table') ? 'table' : 'chart'} size={13}/><span><strong>{visual.title || visual.visual_type_label}</strong><small>{visual.visual_type_label}</small></span></button>
                  <button class="layer-eye" onclick={() => toggleVisualVisibility(i)} title={hiddenVisuals.includes(i) ? 'Show visual' : 'Hide visual'}>{hiddenVisuals.includes(i) ? '○' : '●'}</button>
                </div>
              {/each}
            </div>
          </aside>
        </div>
      {:else if activeView === 'model'}
        <div class="view-layout model-view">
          <aside class="sidepanel">
            <div class="panel-title"><span>TABLES</span><b>{report.tables.length}</b></div>
            <div class="panel-search"><Icon name="search" size={13}/><input bind:value={tableSearch} placeholder="Find table" /></div>
            <label class="system-toggle"><input type="checkbox" bind:checked={hideSystemTables}/> Hide system tables</label>
            <div class="table-list">
              {#each visibleTables() as { table, index }}
                <button class:active={activeTable === index} onclick={() => { activeTable = index; activeColumn = 0; columnSearch = ''; }}><Icon name="table" size={16}/><span>{table.name}</span><b>{table.columns.length}</b></button>
              {:else}<div class="empty-side">No readable schema was packaged in this report.</div>{/each}
            </div>
          </aside>
          <section class="content-view">
            <div class="view-heading"><div><div class="crumb">SEMANTIC MODEL {report.deep_model ? '/ DECODED' : ''}</div><h2>{report.tables[activeTable]?.name || 'Data model'}</h2></div><div class="canvas-meta">{report.tables[activeTable]?.row_count ?? '—'} rows · {report.relationships.length} relationships</div></div>
            {#if report.tables[activeTable]}
              {#if !report.tables[activeTable].columns.length && report.tables[activeTable].description}
                <div class="model-notice"><Icon name="info" size={15}/><span>{report.tables[activeTable].description}</span></div>
              {/if}
              <div class="panel-search column-search"><Icon name="search" size={13}/><input bind:value={columnSearch} placeholder="Find column or measure" /></div>
              <div class="data-card">
                <div class="data-header"><span>Column</span><span>Data type</span><span>Kind</span></div>
                {#each visibleColumns() as { column: col, index }}
                  <button class="data-row" class:active={activeColumn === index} onclick={() => activeColumn = index}><span><Icon name="table" size={14}/>{col.name}</span><span>{col.data_type || '—'}{col.cardinality != null ? ` · ${col.cardinality} distinct` : ''}</span><span class="kind-pill">{col.kind}{col.expression ? ' · fx' : ''}{col.is_hidden ? ' · hidden' : ''}</span></button>
                {:else}<div class="empty-row">No columns exposed in the schema.</div>{/each}
              </div>
              {#if report.tables[activeTable].columns[activeColumn]?.expression}
                <div class="expression-panel"><header><span>{report.tables[activeTable].columns[activeColumn].name}</span><span class="expression-tools"><b>DAX EXPRESSION</b><button onclick={() => copyText(report.tables[activeTable].columns[activeColumn].expression, 'dax')}>{copied === 'dax' ? 'COPIED' : 'COPY DAX'}</button></span></header><pre>{report.tables[activeTable].columns[activeColumn].expression}</pre></div>
              {/if}
              {#if report.relationships.length}
                <h3 class="subheading">Relationships</h3>
                <div class="relation-list">{#each report.relationships as rel}<div class:inactive={!rel.is_active}><span>{rel.from_table}<b>{rel.from_column}</b></span><i></i><span>{rel.to_table}<b>{rel.to_column}</b></span><em>{rel.cardinality || '—'} · {rel.cross_filtering || '—'}</em></div>{/each}</div>
              {/if}
              {#if report.model_metadata && Object.keys(report.model_metadata).length}
                <h3 class="subheading">Advanced model metadata</h3>
                <div class="metadata-grid">
                  {#each Object.entries(report.model_metadata).filter(([key]) => key !== 'decoder') as [key, value]}
                    <details><summary><span>{key.replaceAll('_', ' ')}</span><b>{Array.isArray(value) ? value.length : 'JSON'}</b></summary><pre>{JSON.stringify(value, null, 2)}</pre></details>
                  {/each}
                </div>
              {/if}
            {:else}
              <div class="big-empty"><Icon name="model" size={34}/><h3>Binary semantic model</h3><p>This PBIX stores its model in the VertiPaq DataModel binary. Pages and report metadata remain fully inspectable.</p></div>
            {/if}
          </section>
        </div>
      {:else if activeView === 'data'}
        <div class="view-layout model-view data-view">
          <aside class="sidepanel">
            <div class="panel-title"><span>TABLE DATA</span><b>{report.tables.length}</b></div>
            <div class="panel-search"><Icon name="search" size={13}/><input bind:value={tableSearch} placeholder="Find table" /></div>
            <label class="system-toggle"><input type="checkbox" bind:checked={hideSystemTables}/> Hide system tables</label>
            <div class="table-list">
              {#each visibleTables() as { table, index }}
                <button class:active={dataTable === index} onclick={() => { dataFilter = ''; sortColumn = -1; loadTable(index, 0); }}><Icon name="table" size={16}/><span>{table.name}</span><b>{table.row_count ?? '—'}</b></button>
              {:else}<div class="empty-side">No imported model tables are available.</div>{/each}
            </div>
          </aside>
          <section class="content-view">
            <div class="view-heading"><div><div class="crumb">VERTIPAQ / RAW ROWS</div><h2>{report.tables[dataTable]?.name || 'Table data'}</h2></div>{#if tableData}<div class="heading-actions"><div class="canvas-meta">{tableData.total} rows · showing {tableData.offset + 1}–{Math.min(tableData.offset + tableData.rows.length, tableData.total)}</div><button onclick={copyTablePage}>{copied === 'table-page' ? 'COPIED' : 'COPY PAGE CSV'}</button></div>{/if}</div>
            {#if dataLoading}
              <div class="big-empty"><Icon name="table" size={34}/><h3>Decoding table rows…</h3><p>Reading the local VertiPaq model. Nothing is uploaded.</p></div>
            {:else if dataError}
              <div class="big-empty"><Icon name="warning" size={34}/><h3>Rows unavailable</h3><p>{dataError}</p></div>
            {:else if tableData}
              <div class="data-toolbar"><div class="panel-search"><Icon name="search" size={13}/><input bind:value={dataFilter} placeholder="Filter this page" /></div><span>Raw storage values · click a column to sort</span></div>
              <div class="raw-table-wrap">
                <table class="raw-table"><thead><tr>{#each tableData.columns as column, i}<th><button onclick={() => sortData(i)}>{column}{sortColumn === i ? (sortAscending ? ' ↑' : ' ↓') : ''}</button></th>{/each}</tr></thead><tbody>{#each visibleRows() as row}<tr>{#each row as cell}<td title={String(cell ?? '')}>{cell ?? 'NULL'}</td>{/each}</tr>{/each}</tbody></table>
              </div>
              <div class="pager"><button disabled={dataOffset === 0} onclick={() => loadTable(dataTable, Math.max(0, dataOffset - 100))}>← Previous</button><span>{Math.floor(dataOffset / 100) + 1} / {Math.max(1, Math.ceil(tableData.total / 100))}</span><button disabled={dataOffset + 100 >= tableData.total} onclick={() => loadTable(dataTable, dataOffset + 100)}>Next →</button></div>
            {:else}
              <div class="big-empty"><Icon name="table" size={34}/><h3>Select a model table</h3><p>Imported rows are decoded locally from the PBIX semantic model.</p></div>
            {/if}
          </section>
        </div>
      {:else if activeView === 'queries'}
        <section class="content-view standalone sources-view">
          <div class="view-heading"><div><div class="crumb">DAX WORKBENCH</div><h2>Saved DAX queries</h2></div><div class="canvas-meta">{report.dax_queries?.length || 0} saved query tabs</div></div>
          {#if report.dax_queries?.length}
            <div class="query-notice"><Icon name="info" size={15}/><span>These are queries saved by the report author in Power BI Desktop. They are useful runnable examples, but they do not automatically drive report visuals. Select a visual and choose “View semantic query” for its actual query definition.</span></div>
            <div class="query-explorer dax-query-explorer">
              <aside class="query-list">
                {#each report.dax_queries as query, i}
                  <button class:active={activeDaxQuery === i} onclick={() => activeDaxQuery = i}>
                    <Icon name="search" size={15}/><span><strong>{query.name}{query.is_default ? ' · DEFAULT' : ''}</strong><code>{query.path}</code></span>
                  </button>
                {/each}
              </aside>
              <section class="query-code">
                <header><span><i></i>{report.dax_queries[activeDaxQuery]?.name}.dax</span><div class="code-tools"><label><Icon name="search" size={12}/><input bind:value={querySearch} placeholder="Find in DAX" /></label>{#if querySearch}<b>{(report.dax_queries[activeDaxQuery]?.expression || '').toLowerCase().split(querySearch.toLowerCase()).length - 1} MATCHES</b>{/if}<button class="run-query-button" onclick={() => openDaxRunner(report.dax_queries[activeDaxQuery])}>RUN DAX</button><button onclick={() => copyText(report.dax_queries[activeDaxQuery]?.expression, 'saved-dax')}>{copied === 'saved-dax' ? 'COPIED' : 'COPY DAX'}</button></div></header>
                <div class="query-facts"><span>SAVED AUTHOR QUERY</span><em>UTF-16 DAX extracted directly from the package</em></div>
                <div class="code-lines">{#each (report.dax_queries[activeDaxQuery]?.expression || '').split('\n') as line, i}<div class:match={querySearch && line.toLowerCase().includes(querySearch.toLowerCase())}><span>{i + 1}</span><code>{line || ' '}</code></div>{/each}</div>
              </section>
            </div>
          {:else}
            <div class="big-empty"><Icon name="search" size={34}/><h3>No saved DAX query tabs</h3><p>This is normal when the report author never used Power BI Desktop’s DAX query view. Each data visual can still contain its own semantic query—select one in Report and open “View semantic query.”</p></div>
          {/if}
        </section>
      {:else if activeView === 'dax'}
        <section class="content-view standalone dax-page-view">
          <div class="view-heading"><div><div class="crumb">LIVE AAS / XMLA</div><h2>Run DAX query</h2></div><div class="canvas-meta">Values stay in memory until PBI Lens closes</div></div>
          <div class="dax-runner dax-runner-page">
            <div class="dax-runner-body">
              <section class="runner-config">
                <div class="runner-section-title">SERVER &amp; MODEL</div>
                <label><span>AAS server URL</span><input bind:value={aasServerUrl} placeholder="Enter exact configured server URL" autocomplete="off"/><small>Loaded from the opened PBIT connection when it is packaged there; otherwise left blank.</small></label>
                <label><span>Catalog / model</span><input bind:value={aasCatalog} placeholder="Enter exact catalog name" autocomplete="off"/></label>

                <div class="runner-section-title">AZURE APPLICATION CREDENTIALS</div>
                <label><span>Tenant ID</span><input bind:value={aasTenantId} placeholder="Microsoft Entra tenant UUID" autocomplete="off"/></label>
                <label><span>Client ID</span><input bind:value={aasClientId} placeholder="Analysis Services application client UUID" autocomplete="off"/></label>
                <label><span>Client secret</span><input type="password" bind:value={aasClientSecret} placeholder="Application client secret" autocomplete="off"/><small>Masked and held in memory only. It is never saved to disk or local storage.</small></label>

                <div class="runner-section-title">OPTIONAL ROW-LEVEL SECURITY</div>
                <label><span>AAS role</span><input type="password" bind:value={aasRole} placeholder="Enter exact configured role" autocomplete="off"/><small>Leave Role and CustomData both blank for an unscoped service-principal query. Both fields remain masked in memory until the app closes.</small></label>
                <label><span>CustomData</span><input type="password" bind:value={aasCustomData} placeholder="Paste exact configured CustomData value" autocomplete="off"/><small>This value cannot be inferred from the DAX query and is never persisted.</small></label>
              </section>
              <section class="runner-query">
                <div class="runner-section-title">DAX QUERY</div>
                <textarea bind:value={daxRunQuery} spellcheck="false" aria-label="DAX query" placeholder="Enter DAX query"></textarea>
                {#if daxRunError}<div class="runner-error"><Icon name="warning" size={16}/><span><strong>Query failed</strong>{daxRunError}</span></div>{/if}
                {#if daxRunResult}
                  <div class="runner-result-meta"><span>HTTP {daxRunResult.http_status}</span><span>{daxRunResult.elapsed_ms} ms</span><span>{daxRunResult.rows.length} rows</span></div>
                  {#if daxRunResult.columns.length}
                    <div class="runner-table-wrap"><table><thead><tr>{#each daxRunResult.columns as column}<th>{column}</th>{/each}</tr></thead><tbody>{#each daxRunResult.rows.slice(0, 500) as row}<tr>{#each row as cell}<td>{cell}</td>{/each}</tr>{/each}</tbody></table></div>
                  {:else}<div class="runner-empty-result">The request succeeded but the XMLA response contained no tabular rows.</div>{/if}
                {:else if !daxRunError}
                  <div class="runner-placeholder"><Icon name="info" size={22}/><strong>Ready to query Azure Analysis Services</strong><span>Nothing is sent until you press Run. Results and server errors will appear here.</span></div>
                {/if}
              </section>
            </div>
            <footer><span>Runs directly against the configured AAS model using XMLA.</span><button class="primary" onclick={runDax} disabled={daxRunLoading}>{daxRunLoading ? 'RUNNING…' : 'RUN DAX'}</button></footer>
          </div>
        </section>
      {:else if activeView === 'sources'}
        <section class="content-view standalone sources-view">
          <div class="view-heading"><div><div class="crumb">CONNECTIONS &amp; POWER QUERY</div><h2>Data sources</h2></div><div class="canvas-meta">{report.sources.length} sources · {report.queries.length} queries</div></div>
          <div class="source-grid">
            {#each report.sources as source}
              <article><span class="source-icon"><Icon name="database" size={21}/></span><div><strong>{source.kind || 'Data source'}</strong><p>{source.detail}</p></div><button class="copy-source" onclick={() => copyText(source.detail, source.detail)}>{copied === source.detail ? 'COPIED' : 'COPY'}</button></article>
            {:else}<div class="big-empty" class:compact={report.queries.length}><Icon name="database" size={34}/><h3>No external endpoint discovered</h3><p>The queries may use local inline data, parameters, or a non-literal connection value. Inspect the full M definitions below.</p></div>{/each}
          </div>
          {#if report.queries.length}
            <h3 class="subheading query-heading">Power Query definitions</h3>
            <div class="query-explorer">
              <aside class="query-list">
                {#each report.queries as query, i}
                  <button class:active={activeQuery === i} onclick={() => activeQuery = i}>
                    <Icon name="table" size={15}/><span><strong>{query.name}</strong><code>{query.preview || 'Packaged M query'}</code></span>
                  </button>
                {/each}
              </aside>
              <section class="query-code">
                <header><span><i></i>{report.queries[activeQuery]?.name}.m</span><div class="code-tools"><label><Icon name="search" size={12}/><input bind:value={querySearch} placeholder="Find in query" /></label>{#if querySearch}<b>{(report.queries[activeQuery]?.formula || '').toLowerCase().split(querySearch.toLowerCase()).length - 1} MATCHES</b>{/if}<button onclick={() => copyText(report.queries[activeQuery]?.formula, 'query')}>{copied === 'query' ? 'COPIED' : 'COPY M'}</button></div></header>
                <div class="query-facts">
                  {#each report.queries[activeQuery]?.connectors || [] as connector}<span>{connector}</span>{/each}
                  {#if report.queries[activeQuery]?.has_native_query}<span class="native">NATIVE QUERY</span>{/if}
                  {#each report.queries[activeQuery]?.dependencies || [] as dependency}<button onclick={() => goToQuery(dependency)} title={`Open ${dependency}`}>USES {dependency} →</button>{/each}
                  {#if !report.queries[activeQuery]?.connectors.length && !report.queries[activeQuery]?.dependencies.length}<em>No external connector in this query</em>{/if}
                </div>
                <div class="code-lines">{#each (report.queries[activeQuery]?.formula || '').split('\n') as line, i}<div class:match={querySearch && line.toLowerCase().includes(querySearch.toLowerCase())}><span>{i + 1}</span><code>{line || ' '}</code></div>{/each}</div>
              </section>
            </div>
          {/if}
        </section>
      {:else}
        <section class="content-view standalone">
          <div class="view-heading"><div><div class="crumb">PACKAGE EXPLORER</div><h2>File contents</h2></div><div class="search-box"><Icon name="search" size={15}/><input bind:value={search} placeholder="Filter files" /></div></div>
          <div class="package-table">
            <div class="package-head"><span>Name</span><span>Compressed</span><span>Original</span><span>Ratio</span></div>
            {#each filteredEntries as entry}
              <button class="package-row" onclick={() => inspectEntry(entry)}><span><Icon name="file" size={15}/><b>{entry.name}</b></span><span>{readableSize(entry.compressed_size)}</span><span>{readableSize(entry.size)}</span><span>{entry.size ? Math.round((1 - entry.compressed_size / entry.size) * 100) : 0}%</span></button>
            {/each}
          </div>
        </section>
      {/if}
    </section>
    {#if fieldDialog}
      <dialog open class="field-dialog-backdrop" aria-labelledby="field-dialog-title" onclick={(event) => { if (event.target === event.currentTarget) fieldDialog = null; }}>
        <div class="field-dialog">
          <header><div><div class="crumb">{fieldDialog.kind}</div><strong id="field-dialog-title">{fieldDialog.title}</strong></div><button onclick={() => fieldDialog = null} aria-label="Close details"><Icon name="close" size={17}/></button></header>
          <div class="field-dialog-meta"><span>{fieldDialog.language}</span><p>{fieldDialog.subtitle}</p></div>
          <pre>{fieldDialog.content}</pre>
          <footer><button onclick={() => copyText(fieldDialog.content, 'field-dialog')}>{copied === 'field-dialog' ? 'COPIED' : `COPY ${fieldDialog.language}`}</button><button class="primary" onclick={() => fieldDialog = null}>DONE</button></footer>
        </div>
      </dialog>
    {/if}
    {#if visualExplanation}
      <dialog open class="explain-dialog-backdrop" aria-labelledby="explain-dialog-title" onclick={(event) => { if (event.target === event.currentTarget) visualExplanation = null; }}>
        <div class="explain-dialog">
          <header>
            <div><div class="crumb">EVIDENCE-BACKED VISUAL DEBUGGER</div><strong id="explain-dialog-title">{visualExplanation.title}</strong><span>{visualExplanation.type}</span></div>
            <button onclick={() => visualExplanation = null} aria-label="Close visual explanation"><Icon name="close" size={18}/></button>
          </header>
          <div class="explain-body">
            <section class="explain-summary">
              <span class="confidence inferred">INFERRED SUMMARY</span>
              <p>{visualExplanation.summary}</p>
              <div class="confidence-legend"><span class="confidence exact">EXACT</span><em>stored by Power BI</em><span class="confidence inferred">INFERRED</span><em>assembled from packaged evidence</em><span class="confidence unknown">UNKNOWN</span><em>not provable from this file</em></div>
            </section>

            <section class="explain-card">
              <header><div><span>01</span><strong>Calculation</strong></div><b>{visualExplanation.calculations.length}</b></header>
              <div class="explain-list">
                {#each visualExplanation.calculations as calculation}
                  <article><span class:exact={calculation.confidence === 'exact'} class:unknown={calculation.confidence === 'unknown'} class="confidence">{calculation.confidence.toUpperCase()}</span><strong>{calculation.name}</strong><p>{calculation.origin}</p><code>{calculation.detail}</code>{#if calculation.expression}<details><summary>View DAX definition</summary><pre>{calculation.expression}</pre></details>{/if}</article>
                {:else}<div class="explain-empty">No conventional calculation binding was decoded.</div>{/each}
              </div>
            </section>

            <section class="explain-card">
              <header><div><span>02</span><strong>Effective filters</strong></div><b>{visualExplanation.resolvedFilters.length + visualExplanation.scopedFilters.length}</b></header>
              <div class="explain-list">
                {#if visualExplanation.resolvedFilters.length}
                  <div class="explain-subtitle"><span class="confidence exact">EXACT</span> Cached merged Where</div>
                  {#each visualExplanation.resolvedFilters as filter}<article><strong>{filter.kind || 'Resolved condition'}</strong><code>{filter.expression}</code><p>{filter.note}</p></article>{/each}
                {:else}<article><span class="confidence unknown">UNKNOWN</span><strong>Final merged query</strong><p>Power BI did not package a cached final query for this visual.</p></article>{/if}
                {#if visualExplanation.scopedFilters.length}
                  <div class="explain-subtitle"><span class="confidence exact">EXACT</span> Individual scopes</div>
                  {#each visualExplanation.scopedFilters as filter}<article class:muted={!filter.active}><strong>{filter.scope} · {filter.kind || 'Filter'}</strong><code>{filter.expression}</code>{#if filter.target}<p>{filter.target}</p>{/if}</article>{/each}
                {/if}
              </div>
            </section>

            <section class="explain-card">
              <header><div><span>03</span><strong>Behavior</strong></div><b>{visualExplanation.behaviors.length + visualExplanation.interactions.length}</b></header>
              <div class="explain-list">
                {#each visualExplanation.behaviors as behavior}<article><span class="confidence exact">EXACT</span><strong>{behavior.label}</strong><p>{behavior.value}</p></article>{/each}
                {#each visualExplanation.interactions as interaction}<article><span class="confidence exact">EXACT</span><strong>{interaction.behavior}</strong><p>{interaction.direction} {interaction.other}</p><code>interaction type {interaction.type}</code></article>{/each}
              </div>
            </section>

            <section class="explain-card explain-unknowns">
              <header><div><span>04</span><strong>Unknown or live-model dependent</strong></div><b>{visualExplanation.unknowns.length}</b></header>
              <div class="explain-list">{#each visualExplanation.unknowns as unknown}<article><span class="confidence unknown">UNKNOWN</span><p>{unknown}</p></article>{/each}</div>
            </section>
          </div>
          <footer><span>No business meaning or current value is guessed.</span><button onclick={() => copyText(explanationMarkdown(visualExplanation), 'visual-explanation')}>{copied === 'visual-explanation' ? 'COPIED' : 'COPY DEBUG REPORT'}</button><button class="primary" onclick={() => visualExplanation = null}>DONE</button></footer>
        </div>
      </dialog>
    {/if}
    {#if entryContent || entryLoading}
      <aside class="entry-drawer">
        <header><div><div class="crumb">PACKAGE ENTRY</div><strong>{entryContent?.name || 'Reading…'}</strong></div><button onclick={() => entryContent = null}><Icon name="close" size={17}/></button></header>
        {#if entryLoading}<div class="drawer-loading">Reading packaged content…</div>{:else}<div class="entry-meta"><span>{entryContent.kind}</span>{#if entryContent.truncated}<b>PREVIEW TRUNCATED</b>{/if}</div><pre>{entryContent.content}</pre>{/if}
      </aside>
    {/if}
    {#if dragging}<div class="drop-overlay"><div><Icon name="upload" size={34}/><strong>Drop to open report</strong><span>Your current file will stay in Recents</span></div></div>{/if}
  </main>
{/if}
