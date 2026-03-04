<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy, type PDFPageProxy } from "pdfjs-dist";
  import workerSrc from "pdfjs-dist/build/pdf.worker.min.mjs?url";

  GlobalWorkerOptions.workerSrc = workerSrc;

  export let src: string;
  export let page: number;
  export let zoom: number;
  export let fitToWindow: boolean;
  export let onPageCount: (count: number) => void;
  export let onFitZoom: (zoom: number) => void;
  export let onError: (message: string) => void;

  let containerEl: HTMLDivElement | null = null;
  let canvasEl: HTMLCanvasElement | null = null;
  let pdfDoc: PDFDocumentProxy | null = null;
  let renderTask: ReturnType<PDFPageProxy["render"]> | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let isRendering = false;
  let pendingRender = false;
  let lastSrc = "";

  async function loadPdf(nextSrc: string) {
    if (!nextSrc) return;
    try {
      if (nextSrc === lastSrc && pdfDoc) {
        return;
      }
      lastSrc = nextSrc;
      renderTask?.cancel();
      renderTask = null;
      pdfDoc = await getDocument(nextSrc).promise;
      onPageCount?.(pdfDoc.numPages);
      await renderPage();
    } catch (error) {
      console.error(error);
      onError?.("Failed to load PDF.");
    }
  }

  function getFitScale(pageWidth: number, pageHeight: number) {
    if (!containerEl) return 1;
    if (!pageWidth || !pageHeight) return 1;
    const widthScale = containerEl.clientWidth / pageWidth;
    const heightScale = containerEl.clientHeight / pageHeight;
    if (!Number.isFinite(widthScale) || !Number.isFinite(heightScale)) return 1;
    return Math.min(widthScale, heightScale);
  }

  async function renderPage() {
    if (!pdfDoc || !canvasEl) return;
    if (isRendering) {
      pendingRender = true;
      return;
    }
    isRendering = true;
    pendingRender = false;

    try {
      const safePage = Math.min(Math.max(page, 1), pdfDoc.numPages);
      const pdfPage = await pdfDoc.getPage(safePage);
      const baseViewport = pdfPage.getViewport({ scale: 1 });
      const scale = fitToWindow
        ? getFitScale(baseViewport.width, baseViewport.height)
        : zoom;
      const safeScale = scale > 0 ? scale : 1;

      if (fitToWindow && Number.isFinite(safeScale)) {
        onFitZoom?.(Number(safeScale.toFixed(4)));
      }

      const viewport = pdfPage.getViewport({ scale: safeScale });
      canvasEl.width = Math.floor(viewport.width);
      canvasEl.height = Math.floor(viewport.height);
      const ctx = canvasEl.getContext("2d");
      if (!ctx) return;

      renderTask?.cancel();
      renderTask = pdfPage.render({ canvasContext: ctx, viewport, canvas: canvasEl });
      await renderTask.promise;
    } catch (error) {
      if ((error as { name?: string })?.name !== "RenderingCancelledException") {
        console.error(error);
        onError?.("Failed to render PDF page.");
      }
    } finally {
      isRendering = false;
      if (pendingRender) {
        pendingRender = false;
        renderPage();
      }
    }
  }

  $: if (src) {
    loadPdf(src);
  }

  $: if (pdfDoc) {
    page;
    zoom;
    fitToWindow;
    renderPage();
  }

  onMount(() => {
    resizeObserver = new ResizeObserver(() => {
      if (fitToWindow) {
        renderPage();
      }
    });
    if (containerEl) {
      resizeObserver.observe(containerEl);
    }

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
    };
  });

  onDestroy(() => {
    renderTask?.cancel();
    resizeObserver?.disconnect();
    resizeObserver = null;
    pdfDoc = null;
  });
</script>

<div class="pdf-container" bind:this={containerEl}>
  <canvas class="pdf-canvas" bind:this={canvasEl}></canvas>
</div>
