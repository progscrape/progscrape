function loadZeitgeist(search) {
  const zeitgeistContainer = document.getElementById("zeitgeist");
  zeitgeistContainer.classList.add("loading");
  (async () => {
    let a = await (await fetch("/zeitgeist.json")).json();
    let b = await (await fetch(`/zeitgeist.json?search=${search}`)).json();
    let base = {};
    for (const story of a.stories.by_shard) {
      base[story[0]] = story[1];
    }
    let count = {};
    for (const story of b.stories.by_shard) {
      count[story[0]] = story[1];
    }
    let labels = a.stories.by_shard.map((x) => x[0]);
    let rawData = [];
    for (const label of labels) {
      if (count[label] == 0 || base[label] == 0) {
        rawData.push(0);
      } else {
        rawData.push(count[label] / base[label]);
      }
    }
    while (rawData.length > 12 && labels.length > 12) {
      if (rawData[0] == 0 && rawData[1] == 0 && rawData[2] == 0) {
        rawData.shift();
        labels.shift();
      } else {
        break;
      }
    }
    for (let i = 0; i < rawData.length; i++) {
      if (rawData[i] == 0) {
        rawData[i] = 1e-12;
      }
    }

    // Basic smoothing
    const data = Array(rawData.length);
    for (let i = 0; i < rawData.length; i++) {
      const a = i > 0 ? rawData[i - 1] : rawData[0];
      const b = rawData[i];
      const c = i < rawData.length - 1
        ? rawData[i + 1]
        : rawData[rawData.length - 1];

      data[i] = a * 0.2 + b * 0.6 + c * 0.2;
    }

    const ctx = zeitgeistContainer.getElementsByTagName("canvas")[0];
    zeitgeistContainer.classList.add("loaded");
    const options = {
      type: "line",
      data: {
        labels,
        datasets: [{
          label: "Popularity",
          data,
          borderWidth: 1,
          tension: 0.1,
        }],
      },
      options: {
        elements: {
          point: {
            pointStyle: false,
          },
        },
        plugins: {
          title: {
            display: true,
            text: `Stories tagged '${search}'`,
          },
          legend: {
            display: false,
          },
        },
        scales: {
          y: {
            min: 1e-12,
            ticks: {
              callback: (_) => "",
            },
            beginAtZero: true,
          },
        },
      },
    };
    new Chart(ctx, options);
  })();
}
