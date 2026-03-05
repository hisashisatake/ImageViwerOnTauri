<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import PdfViewer from "$lib/ui/PdfViewer.svelte";

  type ImageItem = {
    name: string;
    url: string;
    size: number;
    type: string;
    lastModified: number;
    source: "blob" | "file";
  };

  export let images: ImageItem[];
  export let currentIndex: number;
  export let zoom: number;
  export let fitToWindow: boolean;
  export let isDragging: boolean;
  export let isLoading: boolean;
  export let statusMessage: string;
  export let errorMessage: string;
  export let fileInput: HTMLInputElement | null;
  export let imageReloadKey: boolean;
  export let pdfPage: number;
  export let pdfPageCount: number;
  export let spreadStartPage: number;
  export let spreadMode: boolean;
  export let spreadEnabled: boolean;
  export let readingDirection: "ltr" | "rtl";

  export let handleFileChange: (event: Event) => void | Promise<void>;
  export let handleDropzoneKey: (event: KeyboardEvent) => void;
  export let handleDragEnter: (event: DragEvent) => void;
  export let handleDragOver: (event: DragEvent) => void;
  export let handleDragLeave: (event: DragEvent) => void;
  export let handleDrop: (event: DragEvent) => void | Promise<void>;
  export let prevImage: () => void;
  export let nextImage: () => void;
  export let zoomIn: () => void;
  export let zoomOut: () => void;
  export let toggleFit: () => void;
  export let setPdfPageCount: (count: number) => void;
  export let prevPdfPage: () => void;
  export let nextPdfPage: () => void;
  export let handlePdfError: (message: string) => void;
  export let updatePdfFitZoom: (zoom: number) => void;
  export let toggleSpreadMode: () => void;
  export let toggleReadingDirection: () => void;
  export let toggleFullscreen: () => void;

  let canvasEl: HTMLDivElement | null = null;
  let imgElPrimary: HTMLImageElement | null = null;
  let leftPanelEl: HTMLDivElement | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let isFabOpen = false;
  let currentItem: ImageItem | null = null;
  let secondaryItem: ImageItem | null = null;
  let leftItem: ImageItem | null = null;
  let rightItem: ImageItem | null = null;
  let isPdf = false;
  let panX = 0;
  let panY = 0;
  let isPanning = false;
  let panStartX = 0;
  let panStartY = 0;
  let panOriginX = 0;
  let panOriginY = 0;

  function updateFitZoom() {
    if (!fitToWindow) return;
    if (!canvasEl || !imgElPrimary) return;
    if (!imgElPrimary.naturalHeight) return;
    const canvasRect = spreadMode && leftPanelEl
      ? leftPanelEl.getBoundingClientRect()
      : canvasEl.getBoundingClientRect();
    const canvasHeight = canvasRect.height;
    const canvasWidth = canvasRect.width;
    if (!canvasHeight || !canvasWidth) return;
    const heightZoom = canvasHeight / imgElPrimary.naturalHeight;
    const widthZoom = canvasWidth / imgElPrimary.naturalWidth;
    const nextZoom = Math.min(heightZoom, widthZoom);
    zoom = Number(nextZoom.toFixed(4));
  }

  onMount(() => {
    resizeObserver = new ResizeObserver(() => {
      updateFitZoom();
    });
    if (canvasEl) {
      resizeObserver.observe(canvasEl);
    }

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
    };
  });

  onDestroy(() => {
    resizeObserver?.disconnect();
    resizeObserver = null;
  });

  $: if (fitToWindow && images[currentIndex]) {
    updateFitZoom();
  }

  $: currentItem = images[currentIndex] ?? null;
  $: secondaryItem = spreadMode ? images[currentIndex + 1] ?? null : null;
  $: isPdf = currentItem
    ? currentItem.type === "application/pdf" || currentItem.name.toLowerCase().endsWith(".pdf")
    : false;
  $: leftItem = spreadMode && readingDirection === "rtl" ? secondaryItem : currentItem;
  $: rightItem = spreadMode && readingDirection === "rtl" ? currentItem : secondaryItem;

  $: if (fitToWindow) {
    panX = 0;
    panY = 0;
  }

  function toggleFab() {
    isFabOpen = !isFabOpen;
  }

  function closeFab() {
    isFabOpen = false;
  }

  function startPan(event: PointerEvent) {
    if (fitToWindow) return;
    if (event.button !== 0) return;
    const target = event.currentTarget as HTMLElement | null;
    target?.setPointerCapture?.(event.pointerId);
    isPanning = true;
    panStartX = event.clientX;
    panStartY = event.clientY;
    panOriginX = panX;
    panOriginY = panY;
  }

  function movePan(event: PointerEvent) {
    if (!isPanning) return;
    if (event.buttons === 0) {
      endPan(event);
      return;
    }
    const deltaX = event.clientX - panStartX;
    const deltaY = event.clientY - panStartY;
    panX = panOriginX + deltaX;
    panY = panOriginY + deltaY;
  }

  function endPan(event: PointerEvent) {
    if (!isPanning) return;
    const target = event.currentTarget as HTMLElement | null;
    target?.releasePointerCapture?.(event.pointerId);
    isPanning = false;
  }
</script>

<main class="app">
  <div
    class:dragging={isDragging}
    class="dropzone"
    role="button"
    tabindex="0"
    aria-label="Image dropzone. Click to open file picker."
    onkeydown={handleDropzoneKey}
    ondragenter={handleDragEnter}
    ondragover={handleDragOver}
    ondragleave={handleDragLeave}
    ondrop={handleDrop}
  >
    <input
      class="file-input"
      bind:this={fileInput}
      type="file"
      multiple
      accept="image/*,.zip,application/zip,.rar,application/x-rar-compressed,.pdf,application/pdf"
      onchange={handleFileChange}
    />
    {#if isLoading}
      <div class="loading-overlay" role="status" aria-live="polite">
        <div class="spinner" aria-hidden="true"></div>
        <span>{statusMessage || "Loading..."}</span>
      </div>
    {/if}
    {#if images.length}
      <div class="viewer">
        <div class="canvas" bind:this={canvasEl}>
          <div
            class:draggable={!fitToWindow}
            class:dragging={isPanning}
            class="canvas-scroll"
            role="application"
            onpointerdown={startPan}
            onpointermove={movePan}
            onpointerup={endPan}
            onpointercancel={endPan}
          >
            <div class="pan-layer" style={`transform: translate(${panX}px, ${panY}px);`}>
              {#if currentItem}
                {#key imageReloadKey}
                  {#if isPdf}
                    <div class:spread={spreadMode} class="spread-view">
                      <div class="spread-panel left" bind:this={leftPanelEl}>
                        <PdfViewer
                          src={currentItem.url}
                          page={readingDirection === "rtl" ? pdfPage + 1 : pdfPage}
                          zoom={zoom}
                          fitToWindow={fitToWindow}
                          onPageCount={setPdfPageCount}
                          onFitZoom={updatePdfFitZoom}
                          onError={handlePdfError}
                        />
                      </div>
                      {#if spreadMode && pdfPage + 1 <= pdfPageCount}
                        <div class="spread-panel right">
                          <PdfViewer
                            src={currentItem.url}
                            page={readingDirection === "rtl" ? pdfPage : pdfPage + 1}
                            zoom={zoom}
                            fitToWindow={fitToWindow}
                            onPageCount={() => {}}
                            onFitZoom={() => {}}
                            onError={handlePdfError}
                          />
                        </div>
                      {/if}
                    </div>
                  {:else if spreadMode}
                    <div class="spread-view">
                      <div class="spread-panel left" bind:this={leftPanelEl}>
                        {#if leftItem}
                          <img
                            class="spread-image left"
                            bind:this={imgElPrimary}
                            src={leftItem.url}
                            alt={leftItem.name}
                            onload={updateFitZoom}
                            style={`transform: translate(0, -50%) scale(${zoom});`}
                          />
                        {/if}
                      </div>
                      <div class="spread-panel right">
                        {#if rightItem}
                          <img
                            class="spread-image right"
                            src={rightItem.url}
                            alt={rightItem.name}
                            onload={updateFitZoom}
                            style={`transform: translate(0, -50%) scale(${zoom});`}
                          />
                        {/if}
                      </div>
                    </div>
                  {:else}
                    <img
                      bind:this={imgElPrimary}
                      src={currentItem.url}
                      alt={currentItem.name}
                      onload={updateFitZoom}
                      style={`transform: translate(-50%, -50%) scale(${zoom});`}
                    />
                  {/if}
                {/key}
              {/if}
            </div>
          </div>
          <div class="nav-layer">
            <button
              class="nav-button left"
              onclick={(event) => {
                event.stopPropagation();
                if (readingDirection === "rtl") {
                  if (isPdf) {
                    nextPdfPage();
                  } else {
                    nextImage();
                  }
                } else if (isPdf) {
                  prevPdfPage();
                } else {
                  prevImage();
                }
              }}
              disabled={readingDirection === "rtl"
                ? (isPdf ? pdfPage >= pdfPageCount : currentIndex >= images.length - 1)
                : (isPdf ? pdfPage <= 1 : currentIndex === 0)}
              aria-label={readingDirection === "rtl"
                ? (isPdf ? "Next page" : "Next image")
                : (isPdf ? "Previous page" : "Previous image")}
            >
              ‹
            </button>
            <button
              class="nav-button right"
              onclick={(event) => {
                event.stopPropagation();
                if (readingDirection === "rtl") {
                  if (isPdf) {
                    prevPdfPage();
                  } else {
                    prevImage();
                  }
                } else if (isPdf) {
                  nextPdfPage();
                } else {
                  nextImage();
                }
              }}
              disabled={readingDirection === "rtl"
                ? (isPdf ? pdfPage <= 1 : currentIndex === 0)
                : (isPdf ? pdfPage >= pdfPageCount : currentIndex >= images.length - 1)}
              aria-label={readingDirection === "rtl"
                ? (isPdf ? "Previous page" : "Previous image")
                : (isPdf ? "Next page" : "Next image")}
            >
              ›
            </button>
          </div>
          {#if currentItem}
            <div class="meta">
              <span>{currentItem?.name ?? ""}</span>
              <span>{currentItem ? Math.round(currentItem.size / 1024) : 0} KB</span>
              {#if isPdf}
                <span class="counter">
                  {pdfPage}{spreadMode && pdfPage + 1 <= pdfPageCount ? `-${pdfPage + 1}` : ""} / {pdfPageCount}
                </span>
              {:else}
                <span class="counter">
                  {currentIndex + 1}{spreadMode && currentIndex + 1 < images.length ? `-${Math.min(images.length, currentIndex + 2)}` : ""} / {images.length}
                </span>
              {/if}
              <span class="zoom">{Math.round(zoom * 100)}%</span>
            </div>
            <div class="status">
              <span>{spreadEnabled ? "Spread" : "Single"}</span>
              <span>{readingDirection === "rtl" ? "Right Opening" : "Left Opening"}</span>
              <span>{fitToWindow ? "Fit" : "Actual Size"}</span>
            </div>
          {/if}
        </div>
      </div>
    {:else}
      <div class="empty">
        <p>Drop image files here.</p>
        <p>Supported: png, jpg, webp, gif, svg, zip, rar, pdf.</p>
        {#if errorMessage}
          <p>{errorMessage}</p>
        {/if}
      </div>
    {/if}

    <div class="fab">
      {#if isFabOpen}
        <div class="fab-menu" role="menu">
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              toggleFullscreen();
              closeFab();
            }}
            role="menuitem"
          >
            Full Screen
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              toggleSpreadMode();
              closeFab();
            }}
            role="menuitem"
          >
            {spreadStartPage >= 1 ? "Single Page" : "Spread"}
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              toggleReadingDirection();
              closeFab();
            }}
            role="menuitem"
          >
            {readingDirection === "rtl" ? "Left Opening" : "Right Opening"}
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              toggleFit();
              closeFab();
            }}
            role="menuitem"
          >
            {fitToWindow ? "Actual Size" : "Fit"}
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              zoomIn();
              closeFab();
            }}
            role="menuitem"
          >
            Zoom In
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              zoomOut();
              closeFab();
            }}
            role="menuitem"
          >
            Zoom Out
          </button>
        </div>
      {/if}
      <button
        class="fab-main"
        onclick={(event) => {
          event.stopPropagation();
          toggleFab();
        }}
        aria-label="Open controls"
        aria-expanded={isFabOpen}
      >
        +
      </button>
    </div>
  </div>
</main>
