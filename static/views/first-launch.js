
    cancel.onclick = () => console.info("cancel");
    submit.onclick = e => {
        const form = document.querySelector("form");
        const formData = new FormData(form);
        const data = Object.fromEntries(formData.entries());
        console.info("form:" + JSON.stringify(data));
        e.preventDefault();
    };
